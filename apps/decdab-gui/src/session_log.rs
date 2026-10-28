// Session log, ported from a contributor patch (OM5NA, issue #2).
//! A compact CSV record of a listening session: one row a second of what the
//! receiver and decoder report, for looking at reception afterwards.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const HEADER: &str = "utc,source,center_hz,sample_rate_hz,locked,mer_db,fibs_ok,fibs,service_sid,rms_dbfs,dropped,description\n";

/// One row's values.
pub struct Row<'a> {
    pub source: &'a str,
    pub center_hz: f64,
    pub rate_hz: f64,
    pub locked: bool,
    pub mer_db: f32,
    pub fibs_ok: u64,
    pub fibs: u64,
    pub sid: Option<u32>,
    pub rms_dbfs: f32,
    pub dropped: u64,
    pub description: &'a str,
}

#[derive(Default)]
pub struct SessionLog {
    output: Option<(BufWriter<File>, PathBuf)>,
    last_row: Option<Instant>,
}

impl SessionLog {
    pub fn is_on(&self) -> bool {
        self.output.is_some()
    }

    pub fn path(&self) -> Option<&Path> {
        self.output.as_ref().map(|(_, p)| p.as_path())
    }

    pub fn start(&mut self) -> Result<PathBuf, String> {
        self.stop();
        let dir = PathBuf::from("captures");
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let path = dir.join(format!("decdab-session-{}.csv", super::utc_stamp()));
        let file = File::create(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut w = BufWriter::new(file);
        w.write_all(HEADER.as_bytes())
            .map_err(|e| format!("{}: {e}", path.display()))?;
        self.output = Some((w, path.clone()));
        Ok(path)
    }

    pub fn stop(&mut self) {
        if let Some((mut w, _)) = self.output.take() {
            let _ = w.flush();
        }
        self.last_row = None;
    }

    /// Append a row if one is due (at most one a second). A write error stops
    /// the log and is returned once.
    pub fn row(&mut self, r: &Row) -> Option<String> {
        if self
            .last_row
            .is_some_and(|t| t.elapsed() < Duration::from_secs(1))
        {
            return None;
        }
        let (w, path) = self.output.as_mut()?;
        let line = format!(
            "{},{},{:.0},{:.0},{},{:.2},{},{},{},{:.2},{},{}\n",
            super::utc_stamp(),
            field(r.source),
            r.center_hz,
            r.rate_hz,
            r.locked,
            r.mer_db,
            r.fibs_ok,
            r.fibs,
            r.sid.map_or(String::new(), |v| format!("{v:08X}")),
            r.rms_dbfs,
            r.dropped,
            field(r.description),
        );
        if let Err(e) = w.write_all(line.as_bytes()).and_then(|()| w.flush()) {
            let msg = format!("{}: {e}", path.display());
            self.stop();
            return Some(msg);
        }
        self.last_row = Some(Instant::now());
        None
    }
}

impl Drop for SessionLog {
    fn drop(&mut self) {
        self.stop();
    }
}

/// A CSV field: quoted if it holds a comma or quote, newlines made spaces.
fn field(s: &str) -> String {
    let s = s.replace(['\n', '\r'], " ");
    if s.contains([',', '"']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s
    }
}

#[cfg(test)]
mod tests {
    use super::field;

    #[test]
    fn quoting() {
        assert_eq!(field("plain"), "plain");
        assert_eq!(field("a,b"), "\"a,b\"");
        assert_eq!(field("say \"hi\"\n"), "\"say \"\"hi\"\" \"");
    }
}
