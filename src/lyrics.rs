//! Тексты песен: синхронные (LRC) и обычные.

#[derive(Debug, Clone)]
pub struct Line {
    pub at: f64,
    pub text: String,
}

#[derive(Debug, Clone)]
pub enum Lyrics {
    Synced(Vec<Line>),
    Plain(Vec<String>),
}

impl Lyrics {
    pub fn parse_lrc(src: &str) -> Self {
        let mut lines = Vec::new();
        for raw in src.lines() {
            let mut rest = raw.trim();
            let mut stamps = Vec::new();
            // Одна строка может нести несколько меток: [00:12.30][01:40.00]текст
            while let Some(body) = rest.strip_prefix('[') {
                let Some(end) = body.find(']') else { break };
                match parse_stamp(&body[..end]) {
                    Some(t) => stamps.push(t),
                    None => break, // [ar:...], [ti:...] и прочие теги
                }
                rest = &body[end + 1..];
            }
            for t in stamps {
                lines.push(Line {
                    at: t,
                    text: rest.trim().to_string(),
                });
            }
        }
        if lines.is_empty() {
            return Lyrics::Plain(src.lines().map(str::to_owned).collect());
        }
        lines.sort_by(|a, b| a.at.total_cmp(&b.at));
        Lyrics::Synced(lines)
    }

    pub fn plain(src: &str) -> Self {
        Lyrics::Plain(src.lines().map(str::to_owned).collect())
    }
}

fn parse_stamp(s: &str) -> Option<f64> {
    let (m, sec) = s.split_once(':')?;
    let m: f64 = m.trim().parse().ok()?;
    let sec: f64 = sec.trim().replace(',', ".").parse().ok()?;
    Some(m * 60.0 + sec)
}

/// Индекс текущей строки для момента `t`.
pub fn current(lines: &[Line], t: f64) -> Option<usize> {
    match lines.partition_point(|l| l.at <= t) {
        0 => None,
        n => Some(n - 1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lrc() {
        let l = Lyrics::parse_lrc("[ar:X]\n[00:01.50]раз\n[00:03.00][00:10.00]два\n[00:05.25] \n");
        let Lyrics::Synced(lines) = l else { panic!() };
        assert_eq!(lines.len(), 4);
        assert_eq!(lines[0].text, "раз");
        assert_eq!(lines[3].at, 10.0);
        assert_eq!(current(&lines, 0.5), None);
        assert_eq!(current(&lines, 4.0), Some(1));
    }
}
