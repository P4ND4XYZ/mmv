use super::*;
use crate::app::{ExternalOperation, ExternalRequestId, ScrollDirection};
use std::{sync::atomic::Ordering, time::Duration};

fn request(id: &str) -> Request {
    Request {
        id: ExternalRequestId(id.into()),
        operation: ExternalOperation::Top,
    }
}
fn receive(server: &PipeServer) -> Incoming {
    server
        .incoming
        .recv_timeout(Duration::from_secs(3))
        .unwrap()
        .unwrap()
}
fn wait_until(mut predicate: impl FnMut() -> bool) {
    let limit = Instant::now() + Duration::from_secs(3);
    while !predicate() {
        assert!(
            Instant::now() < limit,
            "condition did not complete before test deadline"
        );
        thread::sleep(Duration::from_millis(2));
    }
}
fn short() -> Deadline {
    Deadline::after(Duration::from_millis(80))
}

#[test]
fn fixed_stage_deadlines_use_supplied_time_without_retry_extension() {
    let start = Instant::now();
    for (deadline, seconds) in [
        (Deadline::connection(start), 5),
        (Deadline::startup(start), 30),
        (Deadline::response(start), 30),
    ] {
        assert_eq!(deadline.remaining(start), Duration::from_secs(seconds));
        assert!(!deadline.expired(start + Duration::from_secs(seconds - 1)));
        assert!(deadline.expired(start + Duration::from_secs(seconds)));
        assert_eq!(
            deadline.remaining(start + Duration::from_secs(seconds + 1)),
            Duration::ZERO
        );
        // A retry/check changes neither the start nor the end of this stage.
        assert_eq!(
            deadline.remaining(start + Duration::from_secs(2)),
            Duration::from_secs(seconds - 2)
        );
    }
}

#[test]
fn protocol_round_trips_every_public_operation_and_failure_reason() {
    let mut operations = vec![
        ExternalOperation::Open(r"C:\日本語 space\memo.md".into()),
        ExternalOperation::Top,
        ExternalOperation::Bottom,
        ExternalOperation::Back,
        ExternalOperation::Forward,
    ];
    for direction in [
        ScrollDirection::Down,
        ScrollDirection::Up,
        ScrollDirection::HalfDown,
        ScrollDirection::HalfUp,
        ScrollDirection::PageDown,
        ScrollDirection::PageUp,
    ] {
        operations.push(ExternalOperation::Scroll(direction));
    }
    for operation in operations {
        let original = Request {
            id: ExternalRequestId("independent-id".into()),
            operation,
        };
        assert_eq!(
            Request::decode(&original.encode().unwrap()[4..]).unwrap(),
            original
        );
    }
    for outcome in [Ok(()), Err("読み込み失敗".into())] {
        let original = Response {
            id: ExternalRequestId("reply-id".into()),
            outcome,
        };
        assert_eq!(
            Response::decode(&original.encode().unwrap()[4..]).unwrap(),
            original
        );
    }
}

#[test]
fn protocol_rejects_versions_operations_trailing_bytes_and_invalid_fields() {
    let encoded = request("a").encode().unwrap();
    assert_eq!(&encoded[..], &[9, 0, 0, 0, 1, 0, 1, 1, 0, 0, 0, b'a', 7]);
    for bytes in [
        vec![],
        vec![2, 0, 1],
        vec![1, 0, 3],
        vec![1, 0, 1, 255, 255, 255, 255],
        vec![1, 0, 1, 1, 0, 0, 0, 0xff, 7],
    ] {
        assert!(Request::decode(&bytes).is_err());
    }
    for mutation in [0, 1] {
        let mut bytes = encoded[4..].to_vec();
        if mutation == 0 {
            *bytes.last_mut().unwrap() = 255;
        } else {
            bytes.push(0);
        }
        assert!(Request::decode(&bytes).is_err());
    }
    let relative = Request {
        id: ExternalRequestId("a".into()),
        operation: ExternalOperation::Open("relative.md".into()),
    };
    assert!(relative.encode().is_err());
    assert!(request("").encode().is_err());
    let oversized = Response {
        id: ExternalRequestId("a".into()),
        outcome: Err("x".repeat(60001)),
    };
    assert!(oversized.encode().is_err());
    assert!(Response::decode(&[1, 0, 2, 1, 0, 0, 0, b'a', 2]).is_err());
}

#[test]
fn real_pipe_parallel_requests_receive_reverse_order_results_with_original_ids() {
    let namespace = Namespace::isolated();
    let server = PipeServer::bind(&namespace).unwrap();
    let mut clients = Vec::new();
    for id in ["first", "second", "third"] {
        let namespace = namespace.clone();
        clients.push(thread::spawn(move || {
            PipeClient::connect(&namespace, Deadline::connection(Instant::now()))
                .unwrap()
                .request(request(id), Deadline::response(Instant::now()))
                .unwrap()
        }));
    }
    let mut received = vec![receive(&server), receive(&server), receive(&server)];
    received.sort_by_key(|i| i.request.id.0.clone());
    for incoming in received.into_iter().rev() {
        let id = incoming.request.id;
        let reason = format!("read failed for {}", id.0);
        incoming
            .reply
            .send(Response {
                id,
                outcome: Err(reason),
            })
            .unwrap();
    }
    for (client, expected) in clients.into_iter().zip(["first", "second", "third"]) {
        assert_eq!(
            client.join().unwrap(),
            Response {
                id: ExternalRequestId(expected.into()),
                outcome: Err(format!("read failed for {expected}"))
            }
        );
    }
}

#[test]
fn silent_client_does_not_block_accept_and_disconnect_releases_reply_waiter() {
    let namespace = Namespace::isolated();
    let server = PipeServer::bind(&namespace).unwrap();
    let silent = PipeClient::connect(&namespace, Deadline::connection(Instant::now())).unwrap();
    let client = PipeClient::connect(&namespace, Deadline::connection(Instant::now())).unwrap();
    io::write_all(
        &client.pipe,
        &mut request("disconnected").encode().unwrap(),
        &client.stop,
        short(),
    )
    .unwrap();
    let incoming = receive(&server);
    wait_until(|| server.active.load(Ordering::SeqCst) == 2);
    drop(client);
    wait_until(|| server.active.load(Ordering::SeqCst) == 1);
    assert!(
        incoming
            .reply
            .send(Response {
                id: incoming.request.id,
                outcome: Ok(())
            })
            .is_err()
    );
    drop(silent);
    wait_until(|| server.active.load(Ordering::SeqCst) == 0);
}

#[test]
fn real_pipe_rejects_bad_boundaries_and_truncated_requests_without_dispatch() {
    let namespace = Namespace::isolated();
    let server = PipeServer::bind(&namespace).unwrap();
    for mut bytes in [
        vec![0, 0, 0, 0],
        vec![1, 0, 1, 0],
        vec![9, 0, 0, 0, 1, 0],
        vec![3, 0, 0, 0, 2, 0, 1],
    ] {
        let client = PipeClient::connect(&namespace, Deadline::connection(Instant::now())).unwrap();
        io::write_all(&client.pipe, &mut bytes, &client.stop, short()).unwrap();
        drop(client);
    }
    // A valid subsequent request proves the accept loop is still usable.
    let client = PipeClient::connect(&namespace, Deadline::connection(Instant::now())).unwrap();
    io::write_all(
        &client.pipe,
        &mut request("valid").encode().unwrap(),
        &client.stop,
        short(),
    )
    .unwrap();
    assert_eq!(
        receive(&server).request.id,
        ExternalRequestId("valid".into())
    );
    assert!(server.incoming.try_recv().is_err());
}

#[test]
fn timeout_closes_only_client_wait_and_never_resends_or_cancels_app_operation() {
    let namespace = Namespace::isolated();
    let server = PipeServer::bind(&namespace).unwrap();
    let client = PipeClient::connect(&namespace, Deadline::connection(Instant::now())).unwrap();
    let worker = thread::spawn(move || client.request(request("once"), short()));
    let incoming = receive(&server);
    assert_eq!(
        worker.join().unwrap().unwrap_err().kind(),
        stdio::ErrorKind::TimedOut
    );
    wait_until(|| server.active.load(Ordering::SeqCst) == 0);
    assert!(
        incoming
            .reply
            .send(Response {
                id: incoming.request.id,
                outcome: Ok(())
            })
            .is_err()
    );
    assert!(server.incoming.try_recv().is_err());
    assert!(PipeClient::connect(&namespace, Deadline::connection(Instant::now())).is_ok());
}

#[test]
fn shutdown_cancels_pending_accept_reads_and_app_response_waits_and_joins() {
    let namespace = Namespace::isolated();
    let mut server = PipeServer::bind(&namespace).unwrap();
    let silent = PipeClient::connect(&namespace, Deadline::connection(Instant::now())).unwrap();
    let client = PipeClient::connect(&namespace, Deadline::connection(Instant::now())).unwrap();
    io::write_all(
        &client.pipe,
        &mut request("waiting").encode().unwrap(),
        &client.stop,
        short(),
    )
    .unwrap();
    let incoming = receive(&server);
    let start = Instant::now();
    server.shutdown();
    assert!(start.elapsed() < Duration::from_secs(2));
    assert_eq!(server.active.load(Ordering::SeqCst), 0);
    assert!(
        incoming
            .reply
            .send(Response {
                id: incoming.request.id,
                outcome: Ok(())
            })
            .is_err()
    );
    drop((silent, client));
}

#[test]
fn client_rejects_mismatched_response_and_truncated_response() {
    for wrong_id in [true, false] {
        let namespace = Namespace::isolated();
        let pipe = listener(&namespace, true).unwrap();
        let worker = thread::spawn(move || {
            let stop = Stop::new().unwrap();
            let deadline = Deadline::connection(Instant::now());
            io::accept(&pipe, &stop, deadline).unwrap();
            io::read_frame(&pipe, &stop, deadline).unwrap();
            let mut bytes = if wrong_id {
                Response {
                    id: ExternalRequestId("wrong".into()),
                    outcome: Ok(()),
                }
                .encode()
                .unwrap()
            } else {
                vec![9, 0, 0, 0, 1]
            };
            io::write_all(&pipe, &mut bytes, &stop, deadline).unwrap();
            // Allow client to consume before closing; no flush even in tests.
            if wrong_id {
                let _ = io::read_exact(&pipe, &mut [0], &stop, short());
            }
        });
        let error = PipeClient::connect(&namespace, Deadline::connection(Instant::now()))
            .unwrap()
            .request(request("expected"), Deadline::connection(Instant::now()))
            .unwrap_err();
        if wrong_id {
            assert_eq!(error.kind(), stdio::ErrorKind::InvalidData);
        }
        worker.join().unwrap();
    }
}

#[test]
fn preaccept_disconnect_and_cancelled_idle_accept_are_rearmed_deterministically() {
    let namespace = Namespace::isolated();
    let pipe = listener(&namespace, true).unwrap();
    let stop = Stop::new().unwrap();
    // Fixed ordering, not a timing race: close before ConnectNamedPipe is called.
    let early = PipeClient::try_connect(&namespace).unwrap();
    drop(early);
    let error = io::accept(&pipe, &stop, short()).unwrap_err();
    assert_eq!(error.raw_os_error(), Some(ERROR_NO_DATA as i32));
    // Exercise the same recovery branch used by the server accept worker.
    assert!(!accept_next(&pipe, &stop, short()).unwrap());
    // Idle cancellation must drain completion before this same instance is reused.
    assert!(!accept_next(&pipe, &stop, short()).unwrap());
    let worker = thread::spawn(move || {
        assert!(accept_next(&pipe, &stop, Deadline::connection(Instant::now())).unwrap());
        Request::decode(
            &io::read_frame(&pipe, &stop, Deadline::connection(Instant::now())).unwrap(),
        )
        .unwrap()
    });
    let client = PipeClient::connect(&namespace, Deadline::connection(Instant::now())).unwrap();
    io::write_all(
        &client.pipe,
        &mut request("after-rearm").encode().unwrap(),
        &client.stop,
        Deadline::connection(Instant::now()),
    )
    .unwrap();
    assert_eq!(worker.join().unwrap(), request("after-rearm"));
}

#[test]
fn unreading_client_write_is_cancelled_and_drained_on_deadline_and_shutdown() {
    for shutdown in [false, true] {
        let namespace = Namespace::isolated();
        let pipe = listener(&namespace, true).unwrap();
        let client = PipeClient::try_connect(&namespace).unwrap();
        let stop = Arc::new(Stop::new().unwrap());
        io::accept(&pipe, &stop, short()).unwrap();
        let (started, started_rx) = mpsc::channel();
        let worker_stop = stop.clone();
        let worker = thread::spawn(move || {
            let mut buffer = vec![0x42; 1024 * 1024];
            started.send(()).unwrap();
            io::write_all(
                &pipe,
                &mut buffer,
                &worker_stop,
                if shutdown {
                    Deadline::response(Instant::now())
                } else {
                    short()
                },
            )
        });
        started_rx.recv_timeout(Duration::from_secs(3)).unwrap();
        if shutdown {
            stop.signal();
        }
        let error = worker.join().unwrap().unwrap_err();
        assert_eq!(
            error.kind(),
            if shutdown {
                stdio::ErrorKind::Interrupted
            } else {
                stdio::ErrorKind::TimedOut
            }
        );
        drop(client);
    }
}

#[test]
fn shutdown_releases_response_receipt_wait_without_flush() {
    let namespace = Namespace::isolated();
    let mut server = PipeServer::bind(&namespace).unwrap();
    let client = PipeClient::connect(&namespace, Deadline::connection(Instant::now())).unwrap();
    io::write_all(
        &client.pipe,
        &mut request("no-receipt").encode().unwrap(),
        &client.stop,
        Deadline::connection(Instant::now()),
    )
    .unwrap();
    let incoming = receive(&server);
    incoming
        .reply
        .send(Response {
            id: incoming.request.id,
            outcome: Ok(()),
        })
        .unwrap();
    let payload = io::read_frame(
        &client.pipe,
        &client.stop,
        Deadline::connection(Instant::now()),
    )
    .unwrap();
    assert_eq!(Response::decode(&payload).unwrap().outcome, Ok(()));
    // Intentionally keep the peer open without sending its receipt.
    assert_eq!(server.active.load(Ordering::SeqCst), 1);
    let start = Instant::now();
    server.shutdown();
    assert!(start.elapsed() < Duration::from_secs(2));
    assert_eq!(server.active.load(Ordering::SeqCst), 0);
}

#[test]
fn client_sqos_limits_server_to_identification_impersonation() {
    use windows_sys::Win32::{
        Security::{
            GetTokenInformation, RevertToSelf, SecurityIdentification, TOKEN_QUERY,
            TokenImpersonationLevel,
        },
        System::{
            Pipes::ImpersonateNamedPipeClient,
            Threading::{GetCurrentThread, OpenThreadToken},
        },
    };
    struct Revert;
    impl Drop for Revert {
        fn drop(&mut self) {
            unsafe {
                RevertToSelf();
            }
        }
    }
    let namespace = Namespace::isolated();
    let pipe = listener(&namespace, true).unwrap();
    let client = PipeClient::try_connect(&namespace).unwrap();
    let stop = Stop::new().unwrap();
    io::accept(&pipe, &stop, short()).unwrap();
    io::write_all(&client.pipe, &mut [1], &client.stop, short()).unwrap();
    io::read_exact(&pipe, &mut [0], &stop, short()).unwrap();
    assert_ne!(unsafe { ImpersonateNamedPipeClient(pipe.0) }, 0);
    let _revert = Revert;
    let mut raw = ptr::null_mut();
    assert_ne!(
        unsafe { OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, 1, &mut raw) },
        0
    );
    let token = Handle::new(raw).unwrap();
    let mut level = 0u32;
    let mut length = 0;
    assert_ne!(
        unsafe {
            GetTokenInformation(
                token.0,
                TokenImpersonationLevel,
                (&mut level as *mut u32).cast(),
                4,
                &mut length,
            )
        },
        0
    );
    assert_eq!(level, SecurityIdentification as u32);
}

#[test]
fn reply_id_mismatch_does_not_complete_another_request() {
    let namespace = Namespace::isolated();
    let server = PipeServer::bind(&namespace).unwrap();
    let client = PipeClient::connect(&namespace, Deadline::connection(Instant::now())).unwrap();
    let worker = thread::spawn(move || {
        client.request(request("correct"), Deadline::connection(Instant::now()))
    });
    let incoming = receive(&server);
    assert_eq!(
        incoming
            .reply
            .send(Response {
                id: ExternalRequestId("wrong".into()),
                outcome: Ok(())
            })
            .unwrap_err()
            .kind(),
        stdio::ErrorKind::InvalidData
    );
    assert!(worker.join().unwrap().is_err());
}
