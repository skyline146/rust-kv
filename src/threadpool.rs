use std::{
    sync::{Arc, Mutex, mpsc},
    thread::{self, JoinHandle},
};

type Task = Box<dyn FnOnce() + Send + 'static>;

pub struct ThreadPool {
    tx: Option<mpsc::Sender<Task>>,
    workers: Vec<Worker>,
}

impl ThreadPool {
    pub fn new(cap: usize) -> Self {
        assert!(cap > 0, "thread pool capacity must be > 0");

        let (tx, rx) = mpsc::channel();
        let rx = Arc::new(Mutex::new(rx));
        let workers = (0..cap).map(|_| Worker::new(Arc::clone(&rx))).collect();

        ThreadPool {
            tx: Some(tx),
            workers,
        }
    }

    pub fn execute<F>(&self, t: F) -> Result<(), &str>
    where
        F: FnOnce() + Send + 'static,
    {
        self.tx
            .as_ref()
            .unwrap()
            .send(Box::new(t))
            .map_err(|_| "error on send task to pool")
    }
}

impl Drop for ThreadPool {
    fn drop(&mut self) {
        drop(self.tx.take());

        for w in self.workers.drain(..) {
            if w.handler.join().is_err() {
                eprintln!("thread panicked on join");
            }
        }
    }
}

struct Worker {
    handler: JoinHandle<()>,
}

impl Worker {
    fn new(rx: Arc<Mutex<mpsc::Receiver<Task>>>) -> Self {
        let handler = thread::spawn(move || {
            loop {
                let msg = rx.lock().unwrap().recv();
                match msg {
                    Ok(f) => f(),
                    Err(_) => break,
                }
            }
        });

        Worker { handler }
    }
}
