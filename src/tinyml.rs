//! TinyMl — виртуальная машина офлайн-рекомендателя Моей волны (как в мобильном клиенте).
//!
//! Сервер (`rotor/get-offline-recommender`) присылает программу — `int[]` с кодом и данными,
//! собранными под конкретный набор скачанных треков: оценки, соседи похожих и «разнообразных»
//! треков. Машина исполняет её локально: фильтры, оценка, softmax с температурой, реакция на
//! прослушивания, пропуски и лайки. Нейросети на устройстве нет — обучение на сервере.
//!
//! Адресное пространство: `[0, ram)` — память (состояние), дальше — сама программа (только чтение).
//! Инструкция: младшие 7 бит — код, по 3 бита на режим каждого аргумента начиная с 7-го,
//! биты 28–30 — число аргументов; аргументы — следующие слова. Возврат из программы — переход
//! на адрес 0. Семантика повторяет эталон в клиенте один в один (обёртки целых, float через биты).

use std::time::{Instant, SystemTime, UNIX_EPOCH};

const MAGIC: i32 = 1_413_829_430;
const MAX_STEPS: u64 = 16_777_216;
/// Версия VM, которую видит программа (опкод 48) — как у эталона.
const VM_VERSION: i32 = 20_250_901;

#[derive(Debug, thiserror::Error)]
pub enum VmError {
    #[error("TinyMl: {0}")]
    Program(String),
}

fn err<T>(s: impl Into<String>) -> Result<T, VmError> {
    Err(VmError::Program(s.into()))
}

type R<T> = Result<T, VmError>;

pub struct Vm {
    rom: Vec<i32>,
    ram: Vec<i32>,
    /// Адреса регистров и областей из заголовка.
    ip: usize,
    sp: usize,
    io: usize,
    io_size: usize,
    stack: i32,
    code: (i32, i32),
    result: usize,
    stack_limit: i32,
    entries: Vec<i32>,
    log_level: i32,
    rng: i64,
    started: Instant,
    pub steps: u64,
}

impl Vm {
    /// `state` — сохранённая память прошлого запуска: подходит, только если от той же программы.
    pub fn new(program: Vec<i32>, state: Option<Vec<i32>>, log_level: i32) -> R<Self> {
        if program.len() < 64 || program[0] != MAGIC {
            return err("это не программа");
        }
        if program[1] != 0 {
            return err("другая мажорная версия программы");
        }
        let ram_size = program[8];
        if !(128 < ram_size && ram_size < 8_388_608) {
            return err("неверный размер памяти");
        }
        let n_entries = program[18];
        if !(0 < n_entries && n_entries < 128) {
            return err("неверное число точек входа");
        }
        let at = program[17] as usize;
        let entries = program
            .get(at..at + n_entries as usize)
            .ok_or(VmError::Program("точки входа за концом программы".into()))?
            .to_vec();
        let io = program[11] as usize;
        let reuse = state
            .as_ref()
            .is_some_and(|s| s.len() == ram_size as usize && s[..io] == program[..io]);
        let mut vm = Self {
            ip: program[9] as usize,
            sp: program[10] as usize,
            io,
            io_size: program[12] as usize,
            stack: program[13],
            code: (program[14], program[15]),
            result: program[16] as usize,
            stack_limit: program[62],
            entries,
            log_level,
            rng: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_millis() as i64),
            started: Instant::now(),
            steps: 0,
            ram: Vec::new(),
            rom: Vec::new(),
        };
        if reuse {
            vm.ram = state.expect("state");
            vm.rom = program;
        } else {
            let mut ram = vec![0; ram_size as usize];
            ram[..io].copy_from_slice(&program[..io]);
            vm.ram = ram;
            vm.rom = program;
            vm.call(&[], 0)?;
        }
        Ok(vm)
    }

    /// Память — для сохранения между запусками.
    pub fn state(&self) -> &[i32] {
        &self.ram
    }

    fn read(&self, addr: i32) -> R<i32> {
        let a = addr as usize;
        if addr < 0 {
            return err("чтение за границей");
        }
        match self.ram.get(a) {
            Some(v) => Ok(*v),
            None => self.rom.get(a - self.ram.len()).copied().ok_or(VmError::Program("чтение за границей".into())),
        }
    }

    fn write(&mut self, addr: i32, v: i32) -> R<()> {
        match self.ram.get_mut(addr as usize) {
            Some(slot) if addr >= 0 => {
                *slot = v;
                Ok(())
            }
            _ => err("запись за границей памяти"),
        }
    }

    /// Строка с нулём на конце, по байту в слове.
    fn string(&self, addr: i32) -> R<String> {
        let mut bytes = Vec::new();
        for i in 0..8192 {
            let v = self.read(addr + i)?;
            if v == 0 {
                return Ok(String::from_utf8_lossy(&bytes).into_owned());
            }
            bytes.push(v as u8);
        }
        err("строка длиннее 8192")
    }

    fn log(&self, level: i32, msg: impl FnOnce() -> R<String>) -> R<()> {
        if level <= 0 || level > self.log_level {
            return Ok(());
        }
        let m = msg()?;
        match level {
            1 | 2 => tracing::warn!("TinyMl: {m}"),
            3 | 4 => tracing::info!("TinyMl: {m}"),
            _ => tracing::debug!("TinyMl: {m}"),
        }
        Ok(())
    }

    /// Вызов точки входа `entry` с аргументами на стеке; возвращает код выхода программы.
    pub fn call(&mut self, args: &[i32], entry: usize) -> R<i32> {
        let Some(&start) = self.entries.get(entry) else {
            return err("нет такой точки входа");
        };
        let (ip, sp) = (self.ip, self.sp);
        self.ram[ip] = start;
        self.ram[sp] = self.stack;
        let base = self.stack as usize;
        self.ram[base] = 0;
        self.ram
            .get_mut(base + 1..base + 1 + args.len())
            .ok_or(VmError::Program("аргументы не влезли".into()))?
            .copy_from_slice(args);
        self.started = Instant::now();
        self.steps = 0;
        let ram_len = self.ram.len() as i32;
        // Эталон не обнуляет аргументы между инструкциями: «лишние» берутся от прошлой.
        let mut a = [0i32; 7];
        let f = |x: i32| f32::from_bits(x as u32);
        let fb = |x: f32| x.to_bits() as i32;
        loop {
            let pc = self.ram[ip];
            if !(self.code.0 <= pc && pc < self.code.1) {
                if pc == 0 {
                    return Ok(self.ram[self.result]);
                }
                return err(format!("недопустимое состояние на выходе: {pc}"));
            }
            if self.steps >= MAX_STEPS {
                return err("слишком много шагов");
            }
            if self.ram[sp] >= self.stack_limit {
                return err("переполнение стека");
            }
            let at = (pc - ram_len) as usize;
            let w = *self.rom.get(at).ok_or(VmError::Program("код за концом".into()))?;
            let op = w & 127;
            let na = ((w >> 28) & 7) as usize;
            for (j, slot) in a.iter_mut().enumerate().take(na) {
                let x = *self.rom.get(at + 1 + j).ok_or(VmError::Program("код за концом".into()))?;
                let spv = self.ram[sp];
                *slot = match (w >> (j * 3 + 7)) & 7 {
                    0 => x,
                    1 => self.read(x)?,
                    2 => self.read(self.read(x)?)?,
                    3 => x.wrapping_add(spv),
                    4 => self.read(spv.wrapping_add(x))?,
                    5 => self.read(self.read(spv.wrapping_add(x))?)?,
                    m => return err(format!("неизвестный режим аргумента {m}")),
                };
            }
            self.ram[ip] = pc + 1 + na as i32;
            let b = |c: bool| c as i32;
            match op {
                1 => self.write(a[0], a[1].wrapping_add(a[2]))?,
                2 => self.write(a[0], fb(f(a[2]) + f(a[1])))?,
                3 => self.write(a[0], a[1].wrapping_sub(a[2]))?,
                4 => self.write(a[0], fb(f(a[1]) - f(a[2])))?,
                5 => self.write(a[0], a[1].wrapping_mul(a[2]))?,
                6 => self.write(a[0], fb(f(a[2]) * f(a[1])))?,
                7 | 9 if a[2] == 0 => return err("деление на ноль"),
                7 => self.write(a[0], a[1].wrapping_div(a[2]))?,
                8 => self.write(a[0], fb(f(a[1]) / f(a[2])))?,
                9 => self.write(a[0], a[1].wrapping_rem(a[2]))?,
                // `as` в Rust, как и Java: насыщение, NaN → 0.
                10 => self.write(a[0], f(a[1]) as i32)?,
                11 => self.write(a[0], fb(a[1] as f32))?,
                12 => self.write(a[0], b(a[1] > a[2]))?,
                13 => self.write(a[0], b(f(a[1]) > f(a[2])))?,
                14 => self.write(a[0], b(a[1] >= a[2]))?,
                15 => self.write(a[0], b(f(a[1]) >= f(a[2])))?,
                16 => self.write(a[0], b(a[1] == a[2]))?,
                17 => self.write(a[0], b(f(a[1]) == f(a[2])))?,
                18 => self.write(a[0], b(a[1] != a[2]))?,
                19 => self.write(a[0], b(f(a[1]) != f(a[2])))?,
                20 => self.write(a[0], a[1] & a[2])?,
                21 => self.write(a[0], a[1] | a[2])?,
                22 => self.write(a[0], !a[1])?,
                23 => self.write(a[0], a[1].wrapping_shl(a[2] as u32))?,
                24 => self.write(a[0], a[1].wrapping_shr(a[2] as u32))?,
                25 => self.write(a[0], a[1])?,
                26 => self.ram[ip] = a[0],
                27 if a[0] > a[1] => self.ram[ip] = a[2],
                28 if f(a[0]) > f(a[1]) => self.ram[ip] = a[2],
                29 if a[0] >= a[1] => self.ram[ip] = a[2],
                30 if f(a[0]) >= f(a[1]) => self.ram[ip] = a[2],
                27..=30 => {}
                32 => self.write(a[0], fb((f(a[1]) as f64).exp() as f32))?,
                33 => self.write(a[0], fb((f(a[1]) as f64).abs().ln() as f32))?,
                34 => self.write(a[0], fb((f(a[1]) as f64).sin() as f32))?,
                35 => self.write(a[0], fb((f(a[1]) as f64).cos() as f32))?,
                36 => self.write(a[0], fb((f(a[1]) as f64).atan() as f32))?,
                37 => self.log(a[0], || self.string(a[1]))?,
                38 => self.log(a[0], || Ok(format_java(&self.string(a[1])?, &[Arg::I(a[2])])))?,
                39 => self.log(a[0], || Ok(format_java(&self.string(a[1])?, &[Arg::F(f(a[2]))])))?,
                40 => self.log(a[0], || Ok(format_java(&self.string(a[1])?, &[Arg::S(self.string(a[2])?)])))?,
                41 => self.log(a[0], || {
                    Ok(format_java(&self.string(a[1])?, &[Arg::I(a[2]), Arg::I(a[3])]))
                })?,
                42 => {
                    let msg = self.string(a[1])?;
                    if a[0] == 0 {
                        return err(msg);
                    }
                }
                43 => {
                    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
                    self.write(a[0], now as i32)?
                }
                // splitmix64, но со знаковыми сдвигами — как в эталоне на Java.
                44 => {
                    self.rng = self.rng.wrapping_sub(7_046_029_254_386_353_131);
                    let mut z = self.rng;
                    z = (z ^ (z >> 30)).wrapping_mul(-4_658_895_280_553_007_687);
                    z = (z ^ (z >> 27)).wrapping_mul(-7_723_592_293_110_705_685);
                    self.write(a[0], ((z ^ (z >> 31)) & 0x7fff_ffff) as i32)?
                }
                45 => self.rng = a[0] as i64,
                46 => {
                    for k in 0..6 {
                        self.write(a[0] + k as i32, a[1 + k])?;
                    }
                }
                47 => self.write(a[0], self.started.elapsed().as_micros() as i32)?,
                48 => self.write(a[0], VM_VERSION)?,
                49 => self.write(a[0], self.log_level)?,
                64 => self.memcpy(a[0], a[1], a[2])?,
                65 => {
                    for k in 0..a[2] {
                        self.write(a[0].wrapping_add(k.wrapping_mul(a[1])), a[3])?;
                    }
                }
                // 66–103 — векторные операции; эта версия программы их не использует.
                _ => return err(format!("неподдерживаемая команда {op}")),
            }
            self.steps += 1;
        }
    }

    /// `System.arraycopy` из памяти или программы в память; источник не пересекает их границу.
    fn memcpy(&mut self, dst: i32, src: i32, len: i32) -> R<()> {
        if len < 0 || dst < 0 || src < 0 {
            return err("memcpy: отрицательные аргументы");
        }
        let (dst, src, len) = (dst as usize, src as usize, len as usize);
        if dst + len > self.ram.len() {
            return err("memcpy: запись за границей");
        }
        let ram_len = self.ram.len();
        if src < ram_len {
            if src + len > ram_len {
                return err("memcpy: чтение за границей");
            }
            self.ram.copy_within(src..src + len, dst);
        } else {
            let s = src - ram_len;
            let chunk = self.rom.get(s..s + len).ok_or(VmError::Program("memcpy: чтение за границей".into()))?;
            self.ram[dst..dst + len].copy_from_slice(chunk);
        }
        Ok(())
    }

    // ── протокол рекомендателя (как в клиенте) ──────────────────────────────

    fn put(&mut self, at: usize, vals: &[i32]) -> R<()> {
        if at + vals.len() > self.io + self.io_size {
            return err("не влезло в область ввода");
        }
        self.ram[at..at + vals.len()].copy_from_slice(vals);
        Ok(())
    }

    fn expect_ok(&mut self, args: &[i32], entry: usize) -> R<()> {
        match self.call(args, entry)? {
            0 => Ok(()),
            c => err(format!("код выхода {c}")),
        }
    }

    /// Какие треки сейчас можно играть (скачаны).
    pub fn update_available(&mut self, ids: &[i32]) -> R<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let n = ids.len().min(10_000);
        let io = self.io;
        self.put(io, &ids[..n])?;
        self.expect_ok(&[io as i32, n as i32], 3)
    }

    /// Начало волны. `context` — недавние треки (id, альбом), `seeds` — `тип:тег`.
    pub fn start_wave(
        &mut self,
        server_data: &[i32],
        context: &[(i32, Option<i32>)],
        seeds: &[String],
        available: &[i32],
    ) -> R<Start> {
        self.update_available(available)?;
        let io = self.io;
        let mut args = [0i32; 6];
        let len = server_data.len();
        if len > 0 {
            self.put(io, server_data)?;
            args[0] = io as i32;
            args[1] = len as i32;
        }
        let n = context.len().min(256);
        let ctx = &context[context.len() - n..];
        args[2] = (io + len) as i32;
        args[3] = (io + len + n) as i32;
        args[4] = n as i32;
        let ids: Vec<i32> = ctx.iter().map(|c| c.0).collect();
        let albums: Vec<i32> = ctx.iter().map(|c| c.1.unwrap_or(-1)).collect();
        self.put(io + len, &ids)?;
        self.put(io + len + n, &albums)?;
        let seeds = seeds.iter().take(8).cloned().collect::<Vec<_>>().join(";");
        let mut bytes: Vec<i32> = seeds.bytes().take(1023).map(|b| b as i8 as i32).collect();
        bytes.push(0);
        let at = io + len + 2 * n;
        self.put(at, &bytes)?;
        args[5] = at as i32;
        self.expect_ok(&args, 1)?;
        Ok(Start {
            tracks: [self.ram[io], self.ram[io + 1]],
            session: self.string(self.ram[io + 2])?,
            batch: self.string(self.ram[io + 3])?,
        })
    }

    /// Следующий трек после `queued` (уже стоящего в очереди) с учётом событий сессии.
    pub fn next_track(&mut self, events: &[Event], last: i32, queued: i32) -> R<Next> {
        let n = events.len().min(256);
        let ev = &events[events.len() - n..];
        let io = self.io;
        let cols: [Vec<i32>; 4] = [
            ev.iter().map(|e| e.at).collect(),
            ev.iter().map(|e| e.track).collect(),
            ev.iter().map(|e| e.kind as i32).collect(),
            ev.iter().map(|e| e.secs.to_bits() as i32).collect(),
        ];
        let mut args = [last, queued, 0, 0, 0, 0, n as i32];
        for (k, col) in cols.iter().enumerate() {
            let at = io + k * n;
            self.put(at, col)?;
            args[2 + k] = at as i32;
        }
        self.expect_ok(&args, 2)?;
        Ok(Next {
            track: self.ram[io],
            batch: self.string(self.ram[io + 1])?,
        })
    }
}

#[derive(Debug, Clone)]
pub struct Start {
    /// Первые треки волны (могут повторяться или быть 0 — фильтруются по доступным).
    pub tracks: [i32; 2],
    #[allow(dead_code)]
    pub session: String,
    pub batch: String,
}

#[derive(Debug, Clone)]
pub struct Next {
    pub track: i32,
    pub batch: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum EventKind {
    Started = 2,
    Finished = 3,
    Skip = 4,
    Like = 5,
    Dislike = 6,
    Unlike = 7,
    Undislike = 8,
}

/// Событие сессии: время (unix, с), трек, тип, сколько секунд прослушано.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct Event {
    pub at: i32,
    pub track: i32,
    pub kind: EventKind,
    pub secs: f32,
}

impl Event {
    pub fn now(track: i32, kind: EventKind, secs: f32) -> Self {
        let at = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs()) as i32;
        Self { at, track, kind, secs }
    }
}

enum Arg {
    I(i32),
    F(f32),
    S(String),
}

/// Подстановка `%d`/`%s`/`%f` по порядку — для логов программы.
fn format_java(fmt: &str, args: &[Arg]) -> String {
    let mut out = String::new();
    let mut it = args.iter();
    let mut chars = fmt.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        // Флаги и точность пропускаем: для логов неважно.
        let mut spec = String::new();
        while let Some(&n) = chars.peek() {
            chars.next();
            if n.is_ascii_alphabetic() || n == '%' {
                spec.push(n);
                break;
            }
        }
        if spec == "%" {
            out.push('%');
            continue;
        }
        match it.next() {
            Some(Arg::I(v)) => out.push_str(&v.to_string()),
            Some(Arg::F(v)) => out.push_str(&format!("{v:.3}")),
            Some(Arg::S(v)) => out.push_str(v),
            None => {}
        }
    }
    out
}
