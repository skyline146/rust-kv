use std::{
    collections::HashMap,
    io::{self, Read, Write},
    net::Shutdown,
    str::from_utf8,
};

use mio::{
    Events, Interest, Poll, Token,
    event::{Event, Source},
    net::{TcpListener, TcpStream},
};

use crate::tcp::ClientState::{Finished, WaitBoth, WaitReadable, WaitWritable};

const SERVER: Token = Token(1);

pub struct TcpServer {
    lis: TcpListener,
    clients: HashMap<Token, Client>,
    next_token: Token,
    available_tokens: Vec<Token>,
}

impl TcpServer {
    pub fn new(addr: &str) -> Self {
        let addr = addr.parse().expect("addr must be correct ip:port");
        let lis = TcpListener::bind(addr)
            .unwrap_or_else(|e| panic!("failed to create tcp listener: {e}"));

        TcpServer {
            lis,
            clients: HashMap::new(),
            next_token: Token(SERVER.0 + 1),
            available_tokens: Vec::new(),
        }
    }

    pub fn listen_and_serve(&mut self) -> Result<(), io::Error> {
        let mut poll = Poll::new()?;
        let mut events_cap = 1024;
        let mut events = Events::with_capacity(events_cap);

        poll.registry()
            .register(&mut self.lis, SERVER, Interest::READABLE)?;

        loop {
            if self.clients.len() > events.capacity() {
                events_cap *= 2;
                events = Events::with_capacity(events_cap);
            }

            if let Err(err) = poll.poll(&mut events, None) {
                if err.kind() != io::ErrorKind::Interrupted {
                    println!("poll failed: {err}");
                }

                continue;
            }

            for event in events.iter() {
                match event.token() {
                    SERVER => loop {
                        let connection = match self.lis.accept() {
                            Ok((connection, _)) => connection,
                            Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
                                break;
                            }
                            Err(e) => {
                                println!("tcp listener accept failed: {e}");
                                break;
                            }
                        };

                        let token = self.next_token();
                        let mut client = Client::new(connection);
                        poll.registry().register(
                            &mut client,
                            token,
                            Interest::READABLE.add(Interest::WRITABLE),
                        )?;
                        self.clients.insert(token, client);
                    },
                    token => {
                        let client = if let Some(client) = self.clients.get_mut(&token) {
                            client
                        } else {
                            break;
                        };

                        match client.handle(event) {
                            Ok(Finished) => {
                                poll.registry().deregister(client).unwrap();
                                self.remove_client(token);
                            }
                            Ok(WaitReadable) => poll
                                .registry()
                                .reregister(client, token, Interest::READABLE)
                                .unwrap_or_else(|e| {
                                    println!("failed to reregister client to readable: {e}");
                                    self.remove_client(token);
                                }),
                            Ok(WaitWritable) => poll
                                .registry()
                                .reregister(client, token, Interest::WRITABLE)
                                .unwrap_or_else(|e| {
                                    println!("failed to reregister client to writable: {e}");
                                    self.remove_client(token);
                                }),
                            Ok(WaitBoth) => break,
                            Err(e) => {
                                println!("failed to handle client: {e}");
                                break;
                            }
                        }
                    }
                }
            }
        }
    }

    fn next_token(&mut self) -> Token {
        let tok = match self.available_tokens.pop() {
            Some(t) => return t,
            None => self.next_token,
        };

        if usize::from(tok) == usize::MAX {
            self.next_token = SERVER;
        }

        self.next_token = Token(self.next_token.0 + 1);

        tok
    }

    fn remove_client(&mut self, token: Token) {
        self.clients.remove(&token);
        self.available_tokens.push(token);
    }
}

impl Drop for TcpServer {
    fn drop(&mut self) {
        self.clients.drain();
    }
}

enum ClientState {
    WaitReadable,
    WaitWritable,
    WaitBoth,
    Finished,
}

struct Client {
    stream: TcpStream,
    // response: Vec<u8>,
}

impl Client {
    fn new(s: TcpStream) -> Self {
        Client { stream: s }
    }

    fn handle(&mut self, event: &Event) -> io::Result<ClientState> {
        if event.is_readable() {
            const RESIZE_STEP: usize = 1024;
            let mut buf = vec![0; RESIZE_STEP];
            let mut bytes_read = 0;

            loop {
                match self.stream.read(&mut buf[bytes_read..]) {
                    Ok(0) => return Ok(Finished),
                    Ok(n) => {
                        bytes_read += n;

                        if bytes_read == buf.len() {
                            buf.resize(buf.len() + RESIZE_STEP, 0);
                        }
                    }
                    Err(ref err) if err.kind() == io::ErrorKind::WouldBlock => break,
                    Err(ref err) if err.kind() == io::ErrorKind::Interrupted => continue,
                    Err(e) => return Err(e),
                }
            }

            if bytes_read > 0 {
                let data = &buf[..bytes_read];
                if let Ok(str) = from_utf8(data) {
                    println!("received data: {str}");
                } else {
                    println!("Received (none UTF-8) data: {data:?}");
                }
            }
        }

        if event.is_writable() {
            const DATA: &[u8] = b"+PONG\r\n";
            let mut bytes_written = 0;

            loop {
                match self.stream.write(&DATA[bytes_written..]) {
                    Ok(n) if n < DATA.len() => {
                        bytes_written += n;
                    }
                    Ok(_) => return Ok(WaitReadable),
                    Err(ref err) if err.kind() == io::ErrorKind::WouldBlock => {
                        return Ok(WaitWritable);
                    }
                    Err(ref err) if err.kind() == io::ErrorKind::Interrupted => {}
                    Err(err) => return Err(err),
                }
            }
        }

        Ok(WaitBoth)
    }
}

impl Source for Client {
    fn register(
        &mut self,
        registry: &mio::Registry,
        token: Token,
        interests: Interest,
    ) -> io::Result<()> {
        self.stream.register(registry, token, interests)
    }

    fn reregister(
        &mut self,
        registry: &mio::Registry,
        token: Token,
        interests: Interest,
    ) -> io::Result<()> {
        self.stream.reregister(registry, token, interests)
    }

    fn deregister(&mut self, registry: &mio::Registry) -> io::Result<()> {
        self.stream.deregister(registry)
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        if let Err(e) = self.stream.flush() {
            println!("failed to flush client: {e}");
        }

        if let Err(e) = self.stream.shutdown(Shutdown::Both) {
            println!("failed to shutdown client: {e}");
        }
    }
}
