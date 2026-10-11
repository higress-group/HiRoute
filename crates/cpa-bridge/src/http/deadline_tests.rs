//! A slow but continuously progressing socket must still honor one total budget.
use super::*;
use std::net::TcpListener;
use std::sync::mpsc;
use std::thread;
use std::time::Instant;

fn read_request_head(stream: &mut TcpStream) {
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let mut head = Vec::new();
    let mut byte = [0];
    while !head.ends_with(b"\r\n\r\n") {
        assert!(
            head.len() < 4096,
            "test request did not finish its bounded header"
        );
        stream.read_exact(&mut byte).unwrap();
        head.push(byte[0]);
    }
}

#[test]
fn continuous_response_drip_cannot_extend_the_absolute_request_deadline() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (accepted, acceptance) = mpsc::channel();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        read_request_head(&mut stream);
        stream.set_nodelay(true).unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 16\r\n\r\n")
            .unwrap();
        accepted.send(()).unwrap();
        for _ in 0..16 {
            if stream.write_all(b"x").is_err() {
                break;
            }
            thread::sleep(Duration::from_millis(35));
        }
    });
    let started = Instant::now();
    let result = request(LoopbackRequest {
        address,
        method: "GET",
        path: "/healthz",
        authorization: None,
        body: &[],
        timeout: Duration::from_millis(180),
    });
    let elapsed = started.elapsed();
    acceptance.recv_timeout(Duration::from_secs(2)).unwrap();
    server.join().unwrap();
    assert!(
        matches!(result, Err(LoopbackHttpError::Io(ref error)) if matches!(error.kind(), std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock)),
        "continuous read progress reset the request budget; elapsed={elapsed:?}"
    );
    assert!(
        elapsed < Duration::from_millis(500),
        "absolute request budget was not enforced: {elapsed:?}"
    );
}

#[test]
fn complete_response_within_the_absolute_budget_still_succeeds() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        read_request_head(&mut stream);
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}")
            .unwrap();
    });
    let result = request(LoopbackRequest {
        address,
        method: "GET",
        path: "/healthz",
        authorization: None,
        body: &[],
        timeout: Duration::from_secs(2),
    });
    server.join().unwrap();
    let response = result.unwrap();
    assert_eq!(response.status, 200);
    assert_eq!(response.body, b"{}");
}
