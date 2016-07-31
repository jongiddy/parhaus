#![deny(warnings)]
extern crate hyper;
extern crate env_logger;
extern crate num_cpus;

use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use hyper::{Control, Decoder, Encoder, HttpStream, Next, StatusCode};
use hyper::server::{Server, Handler, Request, Response};

fn calculate_ultimate_question(rx: mpsc::Receiver<(Control, mpsc::Sender<&'static [u8]>)>) {
    thread::spawn(move || {
        while let Ok((ctrl, tx)) = rx.recv() {
            thread::sleep(Duration::from_millis(500));
            tx.send(b"42").unwrap();
            ctrl.ready(Next::write()).unwrap();
        }
    });
}

struct DownstreamHandler {
    status: StatusCode,
    text: &'static [u8],
    control: Option<Control>,
    worker_tx: mpsc::Sender<(Control, mpsc::Sender<&'static [u8]>)>,
    worker_rx: Option<mpsc::Receiver<&'static [u8]>>,
}

impl Handler<HttpStream> for DownstreamHandler {
    fn on_request(&mut self, req: Request<HttpStream>) -> Next {
        use hyper::RequestUri;
        let path = match *req.uri() {
            RequestUri::AbsolutePath(ref p) => p,
            RequestUri::AbsoluteUri(ref url) => url.path(),
            _ => ""
        };

        match path {
            "/hello" => {
                self.text = b"Hello, World!";
            },
            "/bye" => {
                self.text = b"Good-bye";
            },
            "/question" => {
                let (tx, rx) = mpsc::channel();
                // queue work on our worker
                self.worker_tx.send((self.control.take().unwrap(), tx)).unwrap();
                // save receive channel for response handling
                self.worker_rx = Some(rx);
                // tell hyper we need to wait until we can continue
                return Next::wait();
            }
            _ => {
                self.status = StatusCode::NotFound;
                self.text = b"Not Found";
            }
        }
        Next::write()
    }

    fn on_request_readable(&mut self, _decoder: &mut Decoder<HttpStream>) -> Next {
        Next::write()
    }

    fn on_response(&mut self, res: &mut Response) -> Next {
        use hyper::header::ContentLength;
        res.set_status(self.status);
        if let Some(rx) = self.worker_rx.take() {
            self.text = rx.recv().unwrap();
        }
        res.headers_mut().set(ContentLength(self.text.len() as u64));
        Next::write()
    }

    fn on_response_writable(&mut self, encoder: &mut Encoder<HttpStream>) -> Next {
        match encoder.try_write(self.text) {
            Ok(Some(n)) => {
                if n == self.text.len() {
                    // all done!
                    Next::end()
                } else {
                    // a partial write!
                    // with a static array, we can just move our pointer
                    // another option could be to store a separate index field
                    self.text = &self.text[n..];
                    // there's still more to write, so ask to write again
                    Next::write()
                }
            },
            Ok(None) => {
                // would block, ask to write again
                Next::write()
            },
            Err(e) => {
                println!("oh noes, we cannot say hello! {}", e);
                // remove (kill) this transport
                Next::remove()
            }
        }
    }
}

fn main() {
    env_logger::init().unwrap();

    let (tx, rx) = mpsc::channel();
    calculate_ultimate_question(rx);
    let addr = "127.0.0.1:0".parse().unwrap();
    let (listening, server) = Server::http(&addr).unwrap()
        .handle(move |ctrl| DownstreamHandler {
            status: StatusCode::Ok,
            text: b"",
            control: Some(ctrl),
            worker_tx: tx.clone(),
            worker_rx: None,
        }).unwrap();

    println!("Listening on http://{}", listening);
    server.run()
}
