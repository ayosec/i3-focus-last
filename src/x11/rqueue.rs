use std::sync::Mutex;

pub type Handler<T> = Box<dyn FnMut(&T) -> bool>;

pub struct Queue<T> {
    queue: Mutex<Vec<Handler<T>>>,
}

impl<T> Queue<T> {
    pub fn new() -> Self {
        Queue {
            queue: Mutex::new(Vec::new()),
        }
    }

    pub fn add(&self, h: Handler<T>) {
        self.queue.lock().unwrap().push(h);
    }

    pub fn take(&self) -> Vec<Handler<T>> {
        let mut queue = self.queue.lock().unwrap();
        std::mem::take(&mut queue)
    }

    pub fn process_queue(&self, data: &T) {
        let requests = self.take();
        let mut readd = Vec::with_capacity(requests.len());

        // Replies from requests.
        for mut request in requests {
            if request(data) {
                // Readd the request if it is not finished.
                readd.push(request);
            }
        }

        self.queue.lock().unwrap().extend(readd);
    }
}
