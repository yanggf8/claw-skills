//! Minimal single-purpose HTTP stub. Serves a scripted sequence of responses so
//! retry behaviour can be asserted without network access.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

pub struct Recorded {
    pub body: String,
}

pub struct Stub {
    pub base_url: String,
    pub requests: Arc<Mutex<Vec<Recorded>>>,
    _shutdown: mpsc::Sender<()>,
}

impl Stub {
    pub fn attempts(&self) -> usize {
        self.requests.lock().unwrap().len()
    }
    pub fn body(&self, i: usize) -> String {
        self.requests.lock().unwrap()[i].body.clone()
    }
}

/// `statuses` is consumed one entry per request. A `None` entry means "hang
/// past the per-attempt timeout" (used to exercise timeout handling); the stub
/// sleeps `hang_ms` then closes without responding.
pub fn start(statuses: Vec<Option<u16>>, hang_ms: u64) -> Stub {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let recorded = Arc::clone(&requests);
    let (tx, rx) = mpsc::channel::<()>();

    std::thread::spawn(move || {
        let mut seq = statuses.into_iter();
        for stream in listener.incoming() {
            // try_recv() returns Err for BOTH Empty and Disconnected, so the old
            // `is_ok()` check could never fire and every stub thread leaked.
            if matches!(rx.try_recv(), Ok(()) | Err(mpsc::TryRecvError::Disconnected)) {
                break;
            }
            let Ok(mut stream) = stream else { break };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut content_length = 0usize;
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 {
                    break;
                }
                let lower = line.to_ascii_lowercase();
                if let Some(v) = lower.strip_prefix("content-length:") {
                    content_length = v.trim().parse().unwrap_or(0);
                }
                if line == "\r\n" || line == "\n" {
                    break;
                }
            }
            let mut body = vec![0u8; content_length];
            let _ = reader.read_exact(&mut body);
            recorded.lock().unwrap().push(Recorded {
                body: String::from_utf8_lossy(&body).to_string(),
            });

            // FAIL CLOSED. Answering 200 to an unscripted request turns "the client
            // retried when it must not" into a green test — the single worst thing a
            // stub can do. 418 is non-retryable and appears in no script.
            match seq.next().unwrap_or(Some(418)) {
                None => {
                    std::thread::sleep(std::time::Duration::from_millis(hang_ms));
                }
                Some(code) => {
                    let _ = write!(
                        stream,
                        "HTTP/1.1 {code} X\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    );
                    let _ = stream.flush();
                }
            }
        }
    });

    Stub {
        base_url: format!("http://{addr}"),
        requests,
        _shutdown: tx,
    }
}
