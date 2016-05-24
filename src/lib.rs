use std::net::UdpSocket;
use std::thread;

fn do_server() {
    let mut socket = UdpSocket::bind("127.0.0.1:10101").unwrap();
    loop {
        let mut buf = [0; 256];
        match socket.recv_from(&mut buf) {
            Ok((amt, src)) => {
                println!("Server got message from {}", src);
                socket.send_to("Hello from the server!".as_bytes(), src).unwrap();
            },
            _ => (),
        };
    }
}

fn do_client() {
    let mut socket = UdpSocket::bind("127.0.0.1:10102").unwrap();
    socket.send_to("Hello from the client!".as_bytes(), "127.0.0.1:10101").unwrap();
    let mut buf = [0; 256];
    let (amt, _) = socket.recv_from(&mut buf).unwrap();
    let response = std::str::from_utf8(&buf[0..amt]).unwrap();
    println!("Server said {}", response);
}

#[test]
fn it_works() {
    thread::spawn(|| {
        do_server();
    });
    thread::spawn(|| {
        do_client();
    }).join();
}
