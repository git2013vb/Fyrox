#[cfg(not(target_arch = "wasm32"))]
use crate::core::futures::executor::ThreadPool;
use std::future::Future;

pub struct TaskPool {
    #[cfg(not(target_arch = "wasm32"))]
    thread_pool: ThreadPool,
}

impl TaskPool {
    pub fn new() -> Self {
        Self {
            #[cfg(not(target_arch = "wasm32"))]
            thread_pool: ThreadPool::new().unwrap(),
        }
    }

    #[cfg(target_arch = "wasm32")]
    pub fn spawn_task<F>(&self, future: F)
    where
        F: Future<Output = ()> + 'static,
    {
        crate::core::wasm_bindgen_futures::spawn_local(future);
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn spawn_task<F>(&self, future: F)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        self.thread_pool.spawn_ok(future);
    }
}
