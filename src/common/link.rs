//! Real machine over a serial port.
//!
//! `Engine` is the protocol (what to send next, what a reply means) and has no I/O, so it is
//! tested without hardware. `Link` runs an `Engine` on a thread that owns the port. `Live` is
//! the UI side: port choice, status, console and the requests the Run tab makes.
//!
//! Lines are sent one at a time: the next goes out after the reply to the last (`ok`). That is
//! slower than character counting but a stop or an error always acts within one line.

use crate::common::machine::Firmware;
use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

/// Something that arrived from the machine or happened to the link.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Sent(String),
    Recv(String),
    /// Position in G-code (work) coordinates.
    Position([f64; 3]),
    /// Controller state as reported: Idle, Run, Hold, Alarm, Home...
    State(String),
    /// The job line with this 0-based line number in the G-code was accepted.
    Acked(usize),
    /// A manual line (jog, home, console) was answered.
    ManualDone,
    JobDone,
    /// The job was aborted or the link failed. Nothing is sent after this.
    Error(String),
    Closed,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Out {
    Job(usize),
    Manual,
    Poll,
}

/// Protocol state. Feed it replies with `on_line`, ask it what to write with `next_write`.
pub struct Engine {
    fw: Firmware,
    job: VecDeque<(usize, String)>,
    job_active: bool,
    paused: bool,
    manual: VecDeque<String>,
    out: Option<Out>,
    /// Replies still to come from lines that were thrown away by a stop (Marlin only).
    ignore_ok: usize,
    /// Lines to send after the controller restarted (GRBL).
    after_reset: Vec<String>,
    wco: [f64; 3],
}

impl Engine {
    pub fn new(fw: Firmware) -> Self {
        Self {
            fw,
            job: VecDeque::new(),
            job_active: false,
            paused: false,
            manual: VecDeque::new(),
            out: None,
            ignore_ok: 0,
            after_reset: vec![],
            wco: [0.0; 3],
        }
    }

    /// Start a job: (line number in the G-code, text). Blank lines must already be gone.
    pub fn load(&mut self, lines: Vec<(usize, String)>) {
        self.job = lines.into();
        self.job_active = !self.job.is_empty();
        self.paused = false;
    }

    pub fn queue(&mut self, line: String) {
        self.manual.push_back(line);
    }

    pub fn job_active(&self) -> bool {
        self.job_active
    }

    /// The next line to write, if the port is free. Manual lines go first and also while paused;
    /// job lines wait while paused.
    pub fn next_write(&mut self) -> Option<(String, bool)> {
        if self.out.is_some() {
            return None;
        }
        if let Some(l) = self.manual.pop_front() {
            self.out = Some(Out::Manual);
            return Some((l, false));
        }
        if self.job_active && !self.paused {
            if let Some((n, l)) = self.job.pop_front() {
                self.out = Some(Out::Job(n));
                return Some((l, false));
            }
        }
        None
    }

    /// Ask for the position. Marlin answers with a line that needs its own `ok` slot, GRBL's `?`
    /// is a single byte answered at any time.
    pub fn poll_bytes(&mut self) -> Option<Vec<u8>> {
        match self.fw {
            Firmware::Grbl => Some(b"?".to_vec()),
            _ if self.out.is_none() => {
                self.out = Some(Out::Poll);
                Some(b"M114\n".to_vec())
            }
            _ => None,
        }
    }

    pub fn hold(&mut self) -> Vec<u8> {
        self.paused = true;
        if self.fw == Firmware::Grbl { b"!".to_vec() } else { vec![] }
    }

    pub fn resume(&mut self) -> Vec<u8> {
        self.paused = false;
        if self.fw == Firmware::Grbl { b"~".to_vec() } else { vec![] }
    }

    /// Stop now: forget the job and everything queued. `safe` (end effector off) is sent once
    /// the controller is ready again. Returns what to write at once.
    pub fn stop(&mut self, safe: Vec<String>) -> Vec<u8> {
        self.job.clear();
        self.job_active = false;
        self.paused = false;
        self.manual.clear();
        let waiting = self.out.take().is_some();
        match self.fw {
            // Feed hold, then soft reset: flushes the planner. The reset answer is the banner.
            Firmware::Grbl => {
                self.after_reset = safe;
                vec![b'!', 0x18]
            }
            _ => {
                // The line in flight and the quick stop each still answer.
                self.ignore_ok = usize::from(waiting) + 1;
                self.manual.extend(safe);
                b"M410\n".to_vec()
            }
        }
    }

    fn abort(&mut self, why: String) -> Vec<Event> {
        self.job.clear();
        self.job_active = false;
        self.manual.clear();
        vec![Event::Error(why)]
    }

    pub fn on_line(&mut self, line: &str) -> Vec<Event> {
        let l = line.trim();
        if l.is_empty() {
            return vec![];
        }
        let mut ev = vec![Event::Recv(l.to_string())];
        let low = l.to_ascii_lowercase();
        if l.starts_with('<') {
            ev.extend(self.grbl_status(l));
        } else if low == "ok" || low.starts_with("ok ") {
            if self.ignore_ok > 0 {
                self.ignore_ok -= 1;
                return ev;
            }
            match self.out.take() {
                Some(Out::Job(n)) => {
                    ev.push(Event::Acked(n));
                    if self.job.is_empty() {
                        self.job_active = false;
                        ev.push(Event::JobDone);
                    }
                }
                Some(Out::Manual) => ev.push(Event::ManualDone),
                _ => {}
            }
        } else if low.starts_with("error") {
            if self.ignore_ok > 0 {
                self.ignore_ok -= 1;
                return ev;
            }
            let was = self.out.take();
            if was == Some(Out::Manual) {
                ev.push(Event::ManualDone);
            }
            if self.job_active {
                ev.extend(self.abort(format!("The machine refused a line: {l}")));
            }
        } else if low.starts_with("alarm") {
            self.out = None;
            ev.push(Event::State("Alarm".into()));
            if self.job_active {
                ev.extend(self.abort(format!("Alarm: {l}")));
            }
        } else if low.starts_with("grbl") {
            // The controller restarted: nothing is in flight any more.
            self.out = None;
            self.manual.extend(self.after_reset.drain(..));
        } else if l.starts_with("X:") {
            if let Some(p) = marlin_pos(l) {
                ev.push(Event::Position(p));
            }
        }
        ev
    }

    fn grbl_status(&mut self, l: &str) -> Vec<Event> {
        let mut ev = vec![];
        let body = l.trim_start_matches('<').trim_end_matches('>');
        let mut parts = body.split('|');
        if let Some(s) = parts.next() {
            ev.push(Event::State(s.split(':').next().unwrap_or(s).to_string()));
        }
        let (mut mpos, mut wpos) = (None, None);
        for p in parts {
            let nums = |v: &str| {
                let n: Vec<f64> = v.split(',').filter_map(|x| x.parse().ok()).collect();
                (n.len() >= 3).then(|| [n[0], n[1], n[2]])
            };
            if let Some(v) = p.strip_prefix("MPos:") {
                mpos = nums(v);
            } else if let Some(v) = p.strip_prefix("WPos:") {
                wpos = nums(v);
            } else if let Some(v) = p.strip_prefix("WCO:") {
                if let Some(w) = nums(v) {
                    self.wco = w;
                }
            }
        }
        if let Some(w) = wpos.or(mpos.map(|m| [0, 1, 2].map(|i| m[i] - self.wco[i]))) {
            ev.push(Event::Position(w));
        }
        ev
    }
}

fn marlin_pos(l: &str) -> Option<[f64; 3]> {
    let head = l.split("Count").next()?;
    let get = |k: &str| {
        head.split_whitespace().find_map(|t| t.strip_prefix(k)).and_then(|v| v.parse::<f64>().ok())
    };
    Some([get("X:")?, get("Y:")?, get("Z:")?])
}

/// What the port does for the link thread.
pub trait Wire: Send {
    fn write(&mut self, data: &[u8]) -> io::Result<()>;
    /// One line, or None if nothing complete arrived within a short time.
    fn read_line(&mut self) -> io::Result<Option<String>>;
}

struct SerialWire {
    port: Box<dyn serialport::SerialPort>,
    buf: Vec<u8>,
}

impl Wire for SerialWire {
    fn write(&mut self, data: &[u8]) -> io::Result<()> {
        self.port.write_all(data)?;
        self.port.flush()
    }

    fn read_line(&mut self) -> io::Result<Option<String>> {
        loop {
            if let Some(i) = self.buf.iter().position(|b| *b == b'\n') {
                let line: Vec<u8> = self.buf.drain(..=i).collect();
                return Ok(Some(String::from_utf8_lossy(&line).trim().to_string()));
            }
            let mut tmp = [0u8; 256];
            match self.port.read(&mut tmp) {
                Ok(0) => return Ok(None),
                Ok(n) => self.buf.extend_from_slice(&tmp[..n]),
                Err(e) if e.kind() == io::ErrorKind::TimedOut => return Ok(None),
                Err(e) => return Err(e),
            }
        }
    }
}

enum Req {
    Job(Vec<(usize, String)>),
    Line(String),
    Hold,
    Resume,
    Stop(Vec<String>),
    Close,
}

/// A line unanswered this long means the machine or the cable is gone.
const REPLY_TIMEOUT: Duration = Duration::from_secs(60);
/// Opening a port usually restarts the controller; it is not listening for a while.
const SETTLE: Duration = Duration::from_millis(1800);

pub struct Link {
    tx: Sender<Req>,
    rx: Receiver<Event>,
    pub firmware: Firmware,
}

impl Link {
    pub fn open(port: &str, baud: u32, fw: Firmware) -> Result<Self, String> {
        let p = serialport::new(port, baud)
            .timeout(Duration::from_millis(40))
            .open()
            .map_err(|e| format!("Cannot open {port}: {e}"))?;
        Ok(Self::with_wire(Box::new(SerialWire { port: p, buf: vec![] }), fw))
    }

    pub fn with_wire(wire: Box<dyn Wire>, fw: Firmware) -> Self {
        let (tx, rq) = mpsc::channel();
        let (eq, rx) = mpsc::channel();
        std::thread::spawn(move || worker(wire, fw, rq, eq));
        Self { tx, rx, firmware: fw }
    }

    pub fn send_job(&self, lines: Vec<(usize, String)>) {
        let _ = self.tx.send(Req::Job(lines));
    }

    pub fn send_line(&self, l: String) {
        let _ = self.tx.send(Req::Line(l));
    }

    pub fn hold(&self) {
        let _ = self.tx.send(Req::Hold);
    }

    pub fn resume(&self) {
        let _ = self.tx.send(Req::Resume);
    }

    pub fn stop(&self, safe: Vec<String>) {
        let _ = self.tx.send(Req::Stop(safe));
    }

    pub fn events(&self) -> Vec<Event> {
        self.rx.try_iter().collect()
    }
}

impl Drop for Link {
    fn drop(&mut self) {
        let _ = self.tx.send(Req::Close);
    }
}

fn worker(mut wire: Box<dyn Wire>, fw: Firmware, rq: Receiver<Req>, eq: Sender<Event>) {
    let mut en = Engine::new(fw);
    let started = Instant::now();
    let (mut last_poll, mut last_send) = (Instant::now(), Instant::now());
    let send = |ev: Vec<Event>| ev.into_iter().all(|e| eq.send(e).is_ok());
    let fail = |e: io::Error| {
        let _ = eq.send(Event::Error(format!("Serial error: {e}")));
        let _ = eq.send(Event::Closed);
    };
    loop {
        // Requests from the UI. Real-time bytes go out at once.
        loop {
            match rq.try_recv() {
                Ok(Req::Job(l)) => en.load(l),
                Ok(Req::Line(l)) => en.queue(l),
                Ok(Req::Hold) => {
                    let b = en.hold();
                    if !b.is_empty() && wire.write(&b).is_err() {
                        return fail(io::Error::other("write failed"));
                    }
                }
                Ok(Req::Resume) => {
                    let b = en.resume();
                    if !b.is_empty() && wire.write(&b).is_err() {
                        return fail(io::Error::other("write failed"));
                    }
                }
                Ok(Req::Stop(safe)) => {
                    let b = en.stop(safe);
                    if let Err(e) = wire.write(&b) {
                        return fail(e);
                    }
                    last_send = Instant::now();
                    if !send(vec![Event::Sent("<stop>".into()), Event::Error("Stopped.".into())]) {
                        return;
                    }
                }
                Ok(Req::Close) | Err(mpsc::TryRecvError::Disconnected) => {
                    let _ = eq.send(Event::Closed);
                    return;
                }
                Err(mpsc::TryRecvError::Empty) => break,
            }
        }
        let settled = started.elapsed() > SETTLE;
        if settled {
            if let Some((l, _)) = en.next_write() {
                if let Err(e) = wire.write(format!("{l}\n").as_bytes()) {
                    return fail(e);
                }
                last_send = Instant::now();
                if !send(vec![Event::Sent(l)]) {
                    return;
                }
            } else if last_poll.elapsed() > Duration::from_millis(250) {
                if let Some(b) = en.poll_bytes() {
                    if let Err(e) = wire.write(&b) {
                        return fail(e);
                    }
                }
                last_poll = Instant::now();
            }
        }
        match wire.read_line() {
            Ok(Some(l)) => {
                if !send(en.on_line(&l)) {
                    return;
                }
            }
            Ok(None) => {}
            Err(e) => return fail(e),
        }
        if en.out.is_some_and(|o| o != Out::Poll) && last_send.elapsed() > REPLY_TIMEOUT {
            en.out = None;
            let _ = send(en.abort("The machine stopped answering.".into()));
        }
    }
}

/// G-code lines to send: comments and blank lines removed, with their original line numbers.
pub fn job_lines(gcode: &str) -> Vec<(usize, String)> {
    gcode
        .lines()
        .enumerate()
        .filter_map(|(i, l)| {
            let mut out = String::new();
            let mut depth = 0;
            for c in l.chars() {
                match c {
                    ';' if depth == 0 => break,
                    '(' => depth += 1,
                    ')' if depth > 0 => depth -= 1,
                    _ if depth == 0 => out.push(c),
                    _ => {}
                }
            }
            let t = out.trim();
            (!t.is_empty()).then(|| (i, t.to_string()))
        })
        .collect()
}

/// A jog of `d` mm at `feed` mm/min, relative to where the tool is.
pub fn jog_lines(fw: Firmware, d: [f64; 3], feed: f64) -> Vec<String> {
    let words: Vec<String> = ["X", "Y", "Z"].iter().zip(d).filter(|(_, v)| v.abs() > 1e-9).map(|(a, v)| format!("{a}{v:.3}")).collect();
    if words.is_empty() {
        return vec![];
    }
    match fw {
        Firmware::Grbl => vec![format!("$J=G21G91{}F{feed:.0}", words.concat())],
        _ => vec!["G91".into(), format!("G1 {} F{feed:.0}", words.join(" ")), "G90".into()],
    }
}

/// UI side of the real machine.
#[derive(Default)]
pub struct Live {
    pub ports: Vec<String>,
    pub scanned: bool,
    pub port: String,
    /// Baud rate picked in the Run tab. 0: the machine's own.
    pub baud: u32,
    link: Option<Link>,
    pub state: String,
    pub pos: Option<[f64; 3]>,
    /// Last job line the machine accepted.
    pub acked: Option<usize>,
    pub log: Vec<String>,
    pub job: bool,
    pub paused: bool,
    pub error: Option<String>,
    /// The "machine will move" question is open.
    pub confirm: bool,
    pub console: String,
    /// Manual lines sent and not yet answered.
    busy: usize,
    /// Distance from continuous jogging not sent yet.
    pending: [f64; 3],
}

impl Live {
    pub fn refresh_ports(&mut self) {
        self.scanned = true;
        self.ports = serialport::available_ports().map(|v| v.into_iter().map(|p| p.port_name).collect()).unwrap_or_default();
        if !self.ports.contains(&self.port) {
            self.port = self.ports.first().cloned().unwrap_or_default();
        }
    }

    pub fn connected(&self) -> bool {
        self.link.is_some()
    }

    pub fn connect(&mut self, baud: u32, fw: Firmware) {
        self.error = None;
        match Link::open(&self.port, baud, fw) {
            Ok(l) => {
                self.link = Some(l);
                self.state = "Connecting".into();
                self.log.clear();
                self.pos = None;
                self.acked = None;
                self.job = false;
                self.paused = false;
                self.busy = 0;
            }
            Err(e) => self.error = Some(e),
        }
    }

    pub fn disconnect(&mut self) {
        self.link = None; // dropping the link closes the port
        self.state.clear();
        self.job = false;
        self.paused = false;
        self.pos = None;
    }

    /// Take in what arrived. Call once per frame.
    pub fn poll(&mut self) {
        let Some(l) = &self.link else { return };
        for e in l.events() {
            match e {
                Event::Sent(s) => self.push_log(format!("> {s}")),
                Event::Recv(s) => self.push_log(s),
                Event::Position(p) => self.pos = Some(p),
                Event::State(s) => self.state = s,
                Event::Acked(n) => self.acked = Some(n),
                Event::ManualDone => self.busy = self.busy.saturating_sub(1),
                Event::JobDone => {
                    self.job = false;
                    self.paused = false;
                }
                Event::Error(m) => {
                    self.job = false;
                    self.paused = false;
                    self.busy = 0;
                    self.error = Some(m);
                }
                Event::Closed => {
                    self.link = None;
                    self.state.clear();
                    self.job = false;
                    self.pos = None;
                    return;
                }
            }
        }
    }

    fn push_log(&mut self, s: String) {
        self.log.push(s);
        if self.log.len() > 300 {
            self.log.remove(0);
        }
    }

    /// Ready for a manual command: connected, not running a job, not in alarm.
    pub fn can_jog(&self) -> bool {
        self.link.is_some() && !self.job && !self.state.eq_ignore_ascii_case("alarm") && !self.state.eq_ignore_ascii_case("connecting")
    }

    pub fn send_manual(&mut self, lines: Vec<String>) {
        if let Some(l) = &self.link {
            for s in lines {
                self.busy += 1;
                l.send_line(s);
            }
        }
    }

    pub fn jog(&mut self, fw: Firmware, d: [f64; 3], feed: f64) {
        if self.can_jog() {
            self.send_manual(jog_lines(fw, d, feed));
        }
    }

    /// Continuous jogging: collect the distance and send it in pieces, one at a time, so that
    /// letting go stops the machine within a piece.
    pub fn jog_run(&mut self, fw: Firmware, d: [f64; 3], feed: f64) {
        if !self.can_jog() {
            return;
        }
        for i in 0..3 {
            self.pending[i] += d[i];
        }
        if self.busy == 0 && self.pending.iter().any(|v| v.abs() > 0.02) {
            let p = std::mem::take(&mut self.pending);
            self.send_manual(jog_lines(fw, p, feed));
        }
    }

    pub fn home(&mut self, fw: Firmware, axes: [bool; 3]) {
        if !self.can_jog() {
            return;
        }
        let ax: String = ["X", "Y", "Z"].iter().zip(axes).filter(|(_, on)| *on).map(|(a, _)| *a).collect::<Vec<_>>().join(" ");
        let line = match fw {
            Firmware::Grbl => "$H".to_string(),
            _ => format!("G28 {ax}"),
        };
        self.send_manual(vec![line]);
    }

    pub fn start_job(&mut self, gcode: &str) {
        let lines = job_lines(gcode);
        if let (Some(l), false) = (&self.link, lines.is_empty()) {
            self.acked = None;
            self.job = true;
            self.paused = false;
            self.error = None;
            l.send_job(lines);
        }
    }

    pub fn pause_resume(&mut self) {
        if let (Some(l), true) = (&self.link, self.job) {
            if self.paused {
                l.resume();
            } else {
                l.hold();
            }
            self.paused = !self.paused;
        }
    }

    pub fn stop(&mut self, safe: Vec<String>) {
        if let Some(l) = &self.link {
            l.stop(safe);
        }
        self.job = false;
        self.paused = false;
        self.pending = [0.0; 3];
    }

    pub fn unlock(&mut self) {
        self.send_manual(vec!["$X".into()]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(n: usize) -> Vec<(usize, String)> {
        (0..n).map(|i| (i * 2, format!("G1 X{i}"))).collect()
    }

    #[test]
    fn job_is_sent_one_line_at_a_time_and_finishes() {
        let mut e = Engine::new(Firmware::Grbl);
        e.load(lines(2));
        assert_eq!(e.next_write().unwrap().0, "G1 X0");
        assert!(e.next_write().is_none()); // waits for ok
        let ev = e.on_line("ok");
        assert!(ev.contains(&Event::Acked(0)));
        assert_eq!(e.next_write().unwrap().0, "G1 X1");
        let ev = e.on_line("ok");
        assert!(ev.contains(&Event::Acked(2)) && ev.contains(&Event::JobDone));
        assert!(!e.job_active());
    }

    #[test]
    fn error_aborts_the_job() {
        let mut e = Engine::new(Firmware::Marlin);
        e.load(lines(3));
        e.next_write();
        let ev = e.on_line("Error:Printer halted");
        assert!(ev.iter().any(|x| matches!(x, Event::Error(_))));
        assert!(e.next_write().is_none());
    }

    #[test]
    fn hold_stops_job_lines_but_not_manual_ones() {
        let mut e = Engine::new(Firmware::Grbl);
        e.load(lines(2));
        assert_eq!(e.hold(), b"!");
        assert!(e.next_write().is_none());
        e.queue("$X".into());
        assert_eq!(e.next_write().unwrap().0, "$X");
        e.on_line("ok");
        assert_eq!(e.resume(), b"~");
        assert_eq!(e.next_write().unwrap().0, "G1 X0");
    }

    #[test]
    fn grbl_stop_resets_then_sends_safe_lines_after_the_banner() {
        let mut e = Engine::new(Firmware::Grbl);
        e.load(lines(3));
        e.next_write();
        assert_eq!(e.stop(vec!["M9".into()]), vec![b'!', 0x18]);
        assert!(e.next_write().is_none());
        e.on_line("Grbl 1.1h ['$' for help]");
        assert_eq!(e.next_write().unwrap().0, "M9");
    }

    #[test]
    fn marlin_stop_ignores_the_late_replies() {
        let mut e = Engine::new(Firmware::Marlin);
        e.load(lines(3));
        e.next_write();
        assert_eq!(e.stop(vec!["M5".into()]), b"M410\n");
        assert!(e.next_write().is_some()); // the safe line goes out
        e.on_line("ok"); // reply of the line that was in flight
        e.on_line("ok"); // reply of M410
        assert!(e.out.is_some(), "the safe line still waits for its own ok");
        assert!(e.on_line("ok").contains(&Event::ManualDone));
    }

    #[test]
    fn status_and_position_parsing() {
        let mut e = Engine::new(Firmware::Grbl);
        let ev = e.on_line("<Run|MPos:10.000,5.000,2.000|FS:500,0|WCO:1.000,1.000,0.000>");
        assert!(ev.contains(&Event::State("Run".into())));
        assert!(ev.contains(&Event::Position([9.0, 4.0, 2.0])));
        let ev = Engine::new(Firmware::Marlin).on_line("X:10.00 Y:20.50 Z:3.00 E:0.00 Count X: 800 Y:1640 Z:1200");
        assert!(ev.contains(&Event::Position([10.0, 20.5, 3.0])));
    }

    #[test]
    fn job_lines_drop_comments_and_keep_numbers() {
        let l = job_lines("; head\nG21 ; mm\n\n(setup) G90\nG1 X1");
        assert_eq!(l, vec![(1, "G21".to_string()), (3, "G90".to_string()), (4, "G1 X1".to_string())]);
    }

    #[test]
    fn jog_lines_per_firmware() {
        assert_eq!(jog_lines(Firmware::Grbl, [1.0, 0.0, -2.5], 1500.0), vec!["$J=G21G91X1.000Z-2.500F1500"]);
        let m = jog_lines(Firmware::Marlin, [0.0, 10.0, 0.0], 600.0);
        assert_eq!(m, vec!["G91", "G1 Y10.000 F600", "G90"]);
    }

    /// A port that answers `ok` to every line, as a controller would.
    struct Echo(std::sync::Arc<std::sync::Mutex<Vec<String>>>, VecDeque<String>);

    impl Wire for Echo {
        fn write(&mut self, data: &[u8]) -> io::Result<()> {
            let t = String::from_utf8_lossy(data).to_string();
            if t.ends_with('\n') {
                self.0.lock().unwrap().push(t.trim().to_string());
                self.1.push_back("ok".into());
            }
            Ok(())
        }

        fn read_line(&mut self) -> io::Result<Option<String>> {
            std::thread::sleep(Duration::from_millis(2));
            Ok(self.1.pop_front())
        }
    }

    #[test]
    fn link_streams_a_job_over_a_wire_and_reports_done() {
        let sent = std::sync::Arc::new(std::sync::Mutex::new(vec![]));
        let link = Link::with_wire(Box::new(Echo(sent.clone(), VecDeque::new())), Firmware::Grbl);
        link.send_job(job_lines("G21\n; c\nG1 X1\n"));
        let mut got = vec![];
        let t0 = Instant::now();
        while !got.contains(&Event::JobDone) && t0.elapsed() < Duration::from_secs(6) {
            got.extend(link.events());
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(got.contains(&Event::JobDone), "{got:?}");
        assert_eq!(*sent.lock().unwrap(), vec!["G21", "G1 X1"]);
    }
}
