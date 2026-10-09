#![warn(clippy::significant_drop_in_scrutinee)]

mod resp;
mod tcp;
mod threadpool;

use crate::tcp::TcpServer;
use crate::threadpool::ThreadPool;

fn main() {
    let _pool = ThreadPool::new(10);
    let mut server = TcpServer::new("127.0.0.1:6379");

    if let Err(e) = server.listen_and_serve() {
        panic!("{e}");
    }
}
