//! Нативный поток PipeWire. Живёт в собственном потоке с `MainLoop`.
//!
//! Формат — F32LE с частотой исходника; через `node.rate` просим граф переключиться
//! на неё (сработает, если частота есть в `default.clock.allowed-rates`), тогда
//! ресемплинга нет вообще.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use color_eyre::eyre::eyre;
use pipewire as pw;
use pw::spa;
use pw::spa::pod::Pod;

use super::{Format, Shared};

enum Msg {
    Configure(Format),
    Active(bool),
    Quit,
}

#[derive(Clone)]
pub struct Handle {
    tx: pw::channel::Sender<Msg>,
    configured: Arc<parking_lot::Mutex<Option<Format>>>,
}

impl Handle {
    /// Пересоздаёт поток под новый формат, если он отличается от текущего.
    pub fn configure(&self, f: Format) {
        let mut cur = self.configured.lock();
        if *cur != Some(f) {
            *cur = Some(f);
            let _ = self.tx.send(Msg::Configure(f));
        }
    }

    /// Пересоздать поток с тем же форматом (сторож при зависшем выводе).
    pub fn reconnect(&self) {
        if let Some(f) = *self.configured.lock() {
            let _ = self.tx.send(Msg::Configure(f));
        }
    }

    pub fn current(&self) -> Option<Format> {
        *self.configured.lock()
    }

    pub fn set_active(&self, active: bool) {
        let _ = self.tx.send(Msg::Active(active));
    }

    pub fn quit(&self) {
        let _ = self.tx.send(Msg::Quit);
    }
}

struct ProcessState {
    shared: Arc<Shared>,
    channels: usize,
    last_vol: f32,
}

struct Live {
    stream: pw::stream::StreamRc,
    _listener: pw::stream::StreamListener<ProcessState>,
}

pub fn spawn(shared: Arc<Shared>) -> color_eyre::Result<Handle> {
    let (tx, rx) = pw::channel::channel::<Msg>();
    let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<(), String>>();
    std::thread::Builder::new().name("pipewire".into()).spawn(move || {
        if let Err(e) = run(shared, rx, &ready_tx) {
            let _ = ready_tx.send(Err(e.to_string()));
        }
    })?;
    ready_rx
        .recv()
        .map_err(|_| eyre!("поток PipeWire умер при старте"))?
        .map_err(|e| eyre!("PipeWire: {e}"))?;
    Ok(Handle {
        tx,
        configured: Arc::new(parking_lot::Mutex::new(None)),
    })
}

fn run(
    shared: Arc<Shared>,
    rx: pw::channel::Receiver<Msg>,
    ready: &std::sync::mpsc::Sender<Result<(), String>>,
) -> Result<(), pw::Error> {
    pw::init();
    let mainloop = pw::main_loop::MainLoopRc::new(None)?;
    let context = pw::context::ContextRc::new(&mainloop, None)?;
    let core = context.connect_rc(None)?;
    let _ = ready.send(Ok(()));

    let live: Rc<RefCell<Option<Live>>> = Rc::new(RefCell::new(None));

    let ml = mainloop.clone();
    let _attached = rx.attach(mainloop.loop_(), {
        let live = live.clone();
        move |msg| match msg {
            Msg::Configure(f) => {
                if let Some(old) = live.borrow_mut().take() {
                    let _ = old.stream.disconnect();
                }
                match make_stream(&core, &shared, f) {
                    Ok(l) => {
                        if shared.paused.load(Ordering::Relaxed) {
                            let _ = l.stream.set_active(false);
                        }
                        *live.borrow_mut() = Some(l);
                    }
                    Err(e) => tracing::error!("не удалось создать поток PipeWire: {e}"),
                }
            }
            Msg::Active(a) => {
                if let Some(l) = live.borrow().as_ref() {
                    let _ = l.stream.set_active(a);
                }
            }
            Msg::Quit => ml.quit(),
        }
    });

    mainloop.run();
    if let Some(l) = live.borrow_mut().take() {
        let _ = l.stream.disconnect();
    }
    Ok(())
}

fn make_stream(core: &pw::core::CoreRc, shared: &Arc<Shared>, f: Format) -> Result<Live, pw::Error> {
    let props = pw::properties::properties! {
        *pw::keys::MEDIA_TYPE => "Audio",
        *pw::keys::MEDIA_CATEGORY => "Playback",
        *pw::keys::MEDIA_ROLE => "Music",
        *pw::keys::APP_NAME => "yamusicrs",
        *pw::keys::NODE_NAME => "yamusicrs",
        *pw::keys::NODE_DESCRIPTION => "Яндекс Музыка (yamusicrs)",
        *pw::keys::AUDIO_CHANNELS => f.channels.to_string(),
        "node.rate" => format!("1/{}", f.rate),
        *pw::keys::NODE_LATENCY => format!("{}/{}", f.rate / 20, f.rate),
    };
    let stream = pw::stream::StreamRc::new(core.clone(), "yamusicrs", props)?;

    let state = ProcessState {
        shared: shared.clone(),
        channels: f.channels as usize,
        last_vol: shared.volume(),
    };
    let listener = stream
        .add_local_listener_with_user_data(state)
        .process(|stream, st| {
            let Some(mut buffer) = stream.dequeue_buffer() else {
                return;
            };
            let requested = buffer.requested() as usize;
            let datas = buffer.datas_mut();
            let data = &mut datas[0];
            let stride = std::mem::size_of::<f32>() * st.channels;
            let Some(slice) = data.data() else { return };
            let mut frames = slice.len() / stride;
            if requested > 0 {
                frames = frames.min(requested);
            }
            let out: &mut [f32] = bytemuck_f32(&mut slice[..frames * stride]);
            fill(st, out);
            let chunk = data.chunk_mut();
            *chunk.offset_mut() = 0;
            *chunk.stride_mut() = stride as i32;
            *chunk.size_mut() = (frames * stride) as u32;
        })
        .register()?;

    let mut info = spa::param::audio::AudioInfoRaw::new();
    info.set_format(spa::param::audio::AudioFormat::F32LE);
    info.set_rate(f.rate);
    info.set_channels(f.channels as u32);
    let mut position = [0; spa::param::audio::MAX_CHANNELS];
    match f.channels {
        1 => position[0] = pw::spa::sys::SPA_AUDIO_CHANNEL_MONO,
        2 => {
            position[0] = pw::spa::sys::SPA_AUDIO_CHANNEL_FL;
            position[1] = pw::spa::sys::SPA_AUDIO_CHANNEL_FR;
        }
        _ => {}
    }
    info.set_position(position);
    let bytes: Vec<u8> = spa::pod::serialize::PodSerializer::serialize(
        std::io::Cursor::new(Vec::new()),
        &spa::pod::Value::Object(spa::pod::Object {
            type_: pw::spa::sys::SPA_TYPE_OBJECT_Format,
            id: pw::spa::sys::SPA_PARAM_EnumFormat,
            properties: info.into(),
        }),
    )
    .expect("pod")
    .0
    .into_inner();
    let mut params = [Pod::from_bytes(&bytes).expect("pod")];

    stream.connect(
        spa::utils::Direction::Output,
        None,
        pw::stream::StreamFlags::AUTOCONNECT
            | pw::stream::StreamFlags::MAP_BUFFERS
            | pw::stream::StreamFlags::RT_PROCESS,
        &mut params,
    )?;
    Ok(Live {
        stream,
        _listener: listener,
    })
}

/// Байтовый буфер PipeWire как `&mut [f32]`. Буферы PipeWire выровнены минимум на 16 байт.
fn bytemuck_f32(bytes: &mut [u8]) -> &mut [f32] {
    assert_eq!(bytes.as_ptr() as usize % std::mem::align_of::<f32>(), 0);
    // SAFETY: выравнивание проверено, длина кратна размеру f32 (кратна stride), любой битовый шаблон валиден для f32.
    unsafe { std::slice::from_raw_parts_mut(bytes.as_mut_ptr().cast::<f32>(), bytes.len() / 4) }
}

fn fill(st: &mut ProcessState, out: &mut [f32]) {
    let shared = &st.shared;
    shared.mark_process();
    let mut cons = shared.cons.lock();

    let avail = cons.slots();
    let take = (avail.min(out.len()) / st.channels) * st.channels;
    let target = shared.volume();
    // Кубическая кривая громкости: на слух линейнее, и 1.0 → множитель ровно 1 (bit-perfect).
    let (from, to) = (curve(st.last_vol), curve(target));
    st.last_vol = target;

    if let Ok(chunk) = cons.read_chunk(take) {
        let (a, b) = chunk.as_slices();
        out[..a.len()].copy_from_slice(a);
        out[a.len()..a.len() + b.len()].copy_from_slice(b);
        chunk.commit_all();
    }
    out[take..].fill(0.0);
    shared.played.fetch_add(take as u64, Ordering::AcqRel);

    if from != 1.0 || to != 1.0 {
        let frames = (take / st.channels).max(1);
        for (i, frame) in out[..take].chunks_mut(st.channels).enumerate() {
            let g = from + (to - from) * (i as f32 / frames as f32);
            for s in frame {
                *s *= g;
            }
        }
    }
}

fn curve(v: f32) -> f32 {
    v * v * v
}
