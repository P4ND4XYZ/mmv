use super::*;
use crate::app::{ExternalOperation, ExternalRequestId, ScrollDirection};
use std::{os::windows::io::AsRawHandle, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    time::timeout,
};

fn request(id: &str) -> Request {
    Request {
        id: ExternalRequestId(id.into()),
        operation: ExternalOperation::Top,
    }
}
fn stage() -> Deadline {
    Deadline::connection(Instant::now())
}
fn short() -> Deadline {
    Deadline::after(Duration::from_millis(80))
}
async fn receive(server: &mut PipeServer) -> Incoming {
    timeout(
        Duration::from_secs(3),
        server.incoming.as_mut().unwrap().recv(),
    )
    .await
    .unwrap()
    .unwrap()
    .unwrap()
}
async fn wait_until(mut predicate: impl FnMut() -> bool) {
    timeout(Duration::from_secs(3), async {
        while !predicate() {
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .expect("condition did not complete before test deadline");
}
async fn send_raw(client: &mut PipeClient, id: &str) {
    io::write_all(&mut client.pipe, &request(id).encode().unwrap(), stage())
        .await
        .unwrap();
}
fn respond(incoming: Incoming, outcome: Result<(), String>) -> stdio::Result<()> {
    incoming.reply.send(Response {
        id: incoming.request.id,
        outcome,
    })
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
    assert!(
        Request {
            id: ExternalRequestId("a".into()),
            operation: ExternalOperation::Open("relative.md".into())
        }
        .encode()
        .is_err()
    );
    assert!(request("").encode().is_err());
    assert!(
        Response {
            id: ExternalRequestId("a".into()),
            outcome: Err("x".repeat(60001))
        }
        .encode()
        .is_err()
    );
    assert!(Response::decode(&[1, 0, 2, 1, 0, 0, 0, b'a', 2]).is_err());
}

#[tokio::test(flavor = "current_thread")]
async fn real_pipe_parallel_requests_receive_reverse_order_results_with_original_ids() {
    let namespace = Namespace::isolated();
    let mut server = PipeServer::bind(&namespace).unwrap();
    let mut clients = Vec::new();
    for id in ["first", "second", "third"] {
        let namespace = namespace.clone();
        clients.push(tokio::spawn(async move {
            PipeClient::connect(&namespace, stage())
                .await
                .unwrap()
                .request(request(id), stage())
                .await
                .unwrap()
        }));
    }
    let mut received = vec![
        receive(&mut server).await,
        receive(&mut server).await,
        receive(&mut server).await,
    ];
    assert_ne!(
        received[0].reply.connection_id(),
        received[1].reply.connection_id()
    );
    received.sort_by_key(|i| i.request.id.0.clone());
    for incoming in received.into_iter().rev() {
        let reason = format!("read failed for {}", incoming.request.id.0);
        respond(incoming, Err(reason)).unwrap();
    }
    for (client, expected) in clients.into_iter().zip(["first", "second", "third"]) {
        assert_eq!(
            client.await.unwrap(),
            Response {
                id: ExternalRequestId(expected.into()),
                outcome: Err(format!("read failed for {expected}"))
            }
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn silent_client_does_not_block_accept_and_disconnect_releases_reply_waiter() {
    let namespace = Namespace::isolated();
    let mut server = PipeServer::bind(&namespace).unwrap();
    let silent = PipeClient::connect(&namespace, stage()).await.unwrap();
    let mut client = PipeClient::connect(&namespace, stage()).await.unwrap();
    send_raw(&mut client, "disconnected").await;
    let incoming = receive(&mut server).await;
    wait_until(|| server.active.load(Ordering::SeqCst) == 2).await;
    drop(client);
    wait_until(|| server.active.load(Ordering::SeqCst) == 1).await;
    assert!(!incoming.reply.is_alive());
    assert!(respond(incoming, Ok(())).is_err());
    drop(silent);
    wait_until(|| server.active.load(Ordering::SeqCst) == 0).await;
}

// Start serve only after a complete request (and its invalid tail) is readable.
// This fixes the ordering rather than depending on the accept/runtime race.
#[tokio::test(flavor = "current_thread")]
async fn already_readable_extra_byte_is_rejected_before_dispatch() {
    let namespace = Namespace::isolated();
    let pipe = listener(&namespace, true).unwrap();
    let mut client = PipeClient::try_connect(&namespace).unwrap();
    pipe.connect().await.unwrap();
    let mut bytes = request("must-not-dispatch").encode().unwrap();
    bytes.push(0x42);
    client.pipe.write_all(&bytes).await.unwrap();
    stage().until(pipe.readable()).await.unwrap();
    let (sender, mut incoming) = mpsc::channel(CAPACITY);
    let peer = Arc::new(Peer {
        id: ConnectionId(1),
        alive: AtomicBool::new(true),
        disconnected: Arc::new(Notify::new()),
    });
    let error = stage()
        .until(serve(pipe, sender, peer.clone()))
        .await
        .unwrap_err();
    assert_eq!(error.kind(), stdio::ErrorKind::InvalidData);
    assert!(
        matches!(
            incoming.try_recv(),
            Err(mpsc::error::TryRecvError::Disconnected)
        ),
        "invalid request reached dispatcher"
    );
    assert!(!peer.alive.load(Ordering::Acquire));
    assert!(io::read_frame(&mut client.pipe, stage()).await.is_err());
}

#[tokio::test(flavor = "current_thread")]
async fn already_disconnected_peer_is_rejected_before_dispatch() {
    let namespace = Namespace::isolated();
    let pipe = listener(&namespace, true).unwrap();
    let mut client = PipeClient::try_connect(&namespace).unwrap();
    pipe.connect().await.unwrap();
    send_raw(&mut client, "must-not-dispatch").await;
    stage().until(pipe.readable()).await.unwrap();
    let raw = client.pipe.as_raw_handle();
    drop(client);
    wait_until(|| {
        let mut flags = 0;
        unsafe { windows_sys::Win32::Foundation::GetHandleInformation(raw, &mut flags) == 0 }
    })
    .await;
    let (sender, mut incoming) = mpsc::channel(CAPACITY);
    let peer = Arc::new(Peer {
        id: ConnectionId(2),
        alive: AtomicBool::new(true),
        disconnected: Arc::new(Notify::new()),
    });
    let error = stage()
        .until(serve(pipe, sender, peer.clone()))
        .await
        .unwrap_err();
    assert!(matches!(
        error.kind(),
        stdio::ErrorKind::BrokenPipe | stdio::ErrorKind::UnexpectedEof
    ));
    assert!(
        matches!(
            incoming.try_recv(),
            Err(mpsc::error::TryRecvError::Disconnected)
        ),
        "departed peer reached dispatcher"
    );
    assert!(!peer.alive.load(Ordering::Acquire));
}

#[tokio::test(flavor = "current_thread")]
async fn extra_data_during_response_wait_closes_connection_and_notifies_without_app_completion() {
    let namespace = Namespace::isolated();
    let mut server = PipeServer::bind(&namespace).unwrap();
    let mut client = PipeClient::connect(&namespace, stage()).await.unwrap();
    send_raw(&mut client, "extra").await;
    let incoming = receive(&mut server).await;
    client.pipe.write_all(&[1]).await.unwrap();
    timeout(
        Duration::from_secs(2),
        server.disconnect_notifications().notified(),
    )
    .await
    .unwrap();
    wait_until(|| !incoming.reply.is_alive()).await;
    assert!(respond(incoming, Ok(())).is_err());
    assert!(io::read_frame(&mut client.pipe, stage()).await.is_err());
}

#[tokio::test(flavor = "current_thread")]
async fn real_pipe_rejects_bad_boundaries_and_truncated_requests_without_dispatch() {
    let namespace = Namespace::isolated();
    let mut server = PipeServer::bind(&namespace).unwrap();
    for bytes in [
        vec![0, 0, 0, 0],
        vec![1, 0, 1, 0],
        vec![9, 0, 0, 0, 1, 0],
        vec![3, 0, 0, 0, 2, 0, 1],
    ] {
        let mut client = PipeClient::connect(&namespace, stage()).await.unwrap();
        client.pipe.write_all(&bytes).await.unwrap();
        // Do not drop until the server detects invalid/truncated input; otherwise
        // teardown cancellation could discard the write we intended to test.
        if bytes.len() >= 4 && u32::from_le_bytes(bytes[..4].try_into().unwrap()) == 9 {
            drop(client);
        } else {
            assert!(io::read_frame(&mut client.pipe, stage()).await.is_err());
        }
    }
    let mut client = PipeClient::connect(&namespace, stage()).await.unwrap();
    send_raw(&mut client, "valid").await;
    assert_eq!(
        receive(&mut server).await.request.id,
        ExternalRequestId("valid".into())
    );
    assert!(server.incoming.as_mut().unwrap().try_recv().is_err());
}

#[tokio::test(flavor = "current_thread")]
async fn timeout_closes_only_client_wait_and_never_resends_or_cancels_app_operation() {
    let namespace = Namespace::isolated();
    let mut server = PipeServer::bind(&namespace).unwrap();
    let client = PipeClient::connect(&namespace, stage()).await.unwrap();
    let worker = tokio::spawn(client.request(request("once"), short()));
    let incoming = receive(&mut server).await;
    assert_eq!(
        worker.await.unwrap().unwrap_err().kind(),
        stdio::ErrorKind::TimedOut
    );
    wait_until(|| server.active.load(Ordering::SeqCst) == 0).await;
    assert!(respond(incoming, Ok(())).is_err());
    assert!(server.incoming.as_mut().unwrap().try_recv().is_err());
    assert!(PipeClient::connect(&namespace, stage()).await.is_ok());
}

#[tokio::test(flavor = "current_thread")]
async fn shutdown_cancels_pending_accept_reads_and_app_response_waits_and_joins() {
    let namespace = Namespace::isolated();
    let mut server = PipeServer::bind(&namespace).unwrap();
    let silent = PipeClient::connect(&namespace, stage()).await.unwrap();
    let mut client = PipeClient::connect(&namespace, stage()).await.unwrap();
    send_raw(&mut client, "waiting").await;
    let incoming = receive(&mut server).await;
    let start = Instant::now();
    server.shutdown();
    server.shutdown();
    assert!(start.elapsed() < Duration::from_secs(2));
    assert_eq!(server.active.load(Ordering::SeqCst), 0);
    assert!(respond(incoming, Ok(())).is_err());
    drop((silent, client));
}

#[tokio::test(flavor = "current_thread")]
async fn client_rejects_mismatched_response_and_truncated_response() {
    for wrong_id in [true, false] {
        let namespace = Namespace::isolated();
        let mut pipe = listener(&namespace, true).unwrap();
        let worker = tokio::spawn(async move {
            pipe.connect().await.unwrap();
            io::read_frame(&mut pipe, stage()).await.unwrap();
            let bytes = if wrong_id {
                Response {
                    id: ExternalRequestId("wrong".into()),
                    outcome: Ok(()),
                }
                .encode()
                .unwrap()
            } else {
                vec![9, 0, 0, 0, 1]
            };
            io::write_all(&mut pipe, &bytes, stage()).await.unwrap();
            // The client either rejects or times out on this incomplete frame.
            let _ = io::read_exact(
                &mut pipe,
                &mut [0],
                Deadline::after(Duration::from_millis(200)),
            )
            .await;
        });
        let error = PipeClient::connect(&namespace, stage())
            .await
            .unwrap()
            .request(request("expected"), short())
            .await
            .unwrap_err();
        assert_eq!(
            error.kind(),
            if wrong_id {
                stdio::ErrorKind::InvalidData
            } else {
                stdio::ErrorKind::TimedOut
            }
        );
        worker.await.unwrap();
    }
}

#[tokio::test(flavor = "current_thread")]
async fn preaccept_disconnect_and_cancelled_idle_accept_are_rearmed_deterministically() {
    let namespace = Namespace::isolated();
    let mut pipe = listener(&namespace, true).unwrap();
    let early = PipeClient::try_connect(&namespace).unwrap();
    let raw = early.pipe.as_raw_handle();
    drop(early);
    // Mio holds its handle until the cancelled background read's IOCP completion.
    // Establish actual OS closure, not just Rust Drop, before ConnectNamedPipe.
    wait_until(|| {
        let mut flags = 0;
        unsafe { windows_sys::Win32::Foundation::GetHandleInformation(raw, &mut flags) == 0 }
    })
    .await;
    // Mio can report a preaccept disconnect either at connect or at the first
    // read, because it eagerly registers a background read. Both paths must
    // reject this peer without decoding a request, then accept the next client.
    match stage().until(pipe.connect()).await {
        Ok(()) => {
            let error = io::read_frame(&mut pipe, stage()).await.unwrap_err();
            assert!(matches!(
                error.kind(),
                stdio::ErrorKind::BrokenPipe | stdio::ErrorKind::UnexpectedEof
            ));
        }
        Err(error) => assert!(matches!(
            error.kind(),
            stdio::ErrorKind::BrokenPipe | stdio::ErrorKind::UnexpectedEof
        )),
    }
    let replacement = listener(&namespace, false).unwrap();
    drop(pipe);
    // Tokio's connect future is cancellation-safe, and the service replaces a
    // pre-disconnected instance without ever relinquishing the secure pipe name.
    assert!(short().until(replacement.connect()).await.is_err());
    let worker = tokio::spawn(async move {
        replacement.connect().await.unwrap();
        let mut replacement = replacement;
        Request::decode(&io::read_frame(&mut replacement, stage()).await.unwrap()).unwrap()
    });
    let mut client = PipeClient::connect(&namespace, stage()).await.unwrap();
    send_raw(&mut client, "after-rearm").await;
    assert_eq!(worker.await.unwrap(), request("after-rearm"));
}

#[tokio::test(flavor = "current_thread")]
async fn unreading_client_write_is_cancelled_on_deadline_and_releases_kernel_handle() {
    use windows_sys::Win32::Foundation::GetHandleInformation;
    let namespace = Namespace::isolated();
    let mut pipe = listener(&namespace, true).unwrap();
    let _client = PipeClient::try_connect(&namespace).unwrap();
    pipe.connect().await.unwrap();
    // Mio queues the first write and reports it accepted before kernel completion.
    pipe.write_all(&vec![0x42; 1024 * 1024]).await.unwrap();
    let error = io::write_all(&mut pipe, &[1], short()).await.unwrap_err();
    assert_eq!(error.kind(), stdio::ErrorKind::TimedOut);
    let raw = pipe.as_raw_handle();
    drop(pipe); // Wrapper cancels even Mio's internally retained buffered write.
    wait_until(|| {
        let mut flags = 0;
        unsafe { GetHandleInformation(raw, &mut flags) == 0 }
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn partial_read_timeout_closes_connection_without_restarting_frame() {
    let namespace = Namespace::isolated();
    let mut pipe = listener(&namespace, true).unwrap();
    let mut client = PipeClient::try_connect(&namespace).unwrap();
    pipe.connect().await.unwrap();
    client.pipe.write_all(&[9, 0]).await.unwrap();
    assert_eq!(
        io::read_frame(&mut pipe, short()).await.unwrap_err().kind(),
        stdio::ErrorKind::TimedOut
    );
    drop(pipe);
    assert!(client.pipe.read_u8().await.is_err());
}

#[tokio::test(flavor = "current_thread")]
async fn shutdown_releases_response_receipt_wait_without_flush() {
    let namespace = Namespace::isolated();
    let mut server = PipeServer::bind(&namespace).unwrap();
    let mut client = PipeClient::connect(&namespace, stage()).await.unwrap();
    send_raw(&mut client, "no-receipt").await;
    respond(receive(&mut server).await, Ok(())).unwrap();
    let payload = io::read_frame(&mut client.pipe, stage()).await.unwrap();
    assert_eq!(Response::decode(&payload).unwrap().outcome, Ok(()));
    assert_eq!(server.active.load(Ordering::SeqCst), 1);
    let start = Instant::now();
    server.shutdown();
    assert!(start.elapsed() < Duration::from_secs(2));
    assert_eq!(server.active.load(Ordering::SeqCst), 0);
}

#[tokio::test(flavor = "current_thread")]
async fn client_sqos_limits_server_to_identification_and_handles_are_noninheritable() {
    use windows_sys::Win32::{
        Foundation::{GetHandleInformation, HANDLE_FLAG_INHERIT},
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
    let mut pipe = listener(&namespace, true).unwrap();
    let mut client = PipeClient::try_connect(&namespace).unwrap();
    pipe.connect().await.unwrap();
    io::write_all(&mut client.pipe, &[1], stage())
        .await
        .unwrap();
    io::read_exact(&mut pipe, &mut [0], stage()).await.unwrap();
    for raw in [pipe.as_raw_handle(), client.pipe.as_raw_handle()] {
        let mut flags = 0;
        assert_ne!(unsafe { GetHandleInformation(raw, &mut flags) }, 0);
        assert_eq!(flags & HANDLE_FLAG_INHERIT, 0);
    }
    assert_ne!(
        unsafe { ImpersonateNamedPipeClient(pipe.as_raw_handle()) },
        0
    );
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

#[tokio::test(flavor = "current_thread")]
async fn reply_id_mismatch_does_not_complete_another_request() {
    let namespace = Namespace::isolated();
    let mut server = PipeServer::bind(&namespace).unwrap();
    let client = PipeClient::connect(&namespace, stage()).await.unwrap();
    let worker = tokio::spawn(client.request(request("correct"), stage()));
    let incoming = receive(&mut server).await;
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
    assert!(worker.await.unwrap().is_err());
}

#[tokio::test(flavor = "current_thread")]
async fn full_request_queue_rejects_new_request_and_disconnect_notification_is_not_lost() {
    let namespace = Namespace::isolated();
    let mut server = PipeServer::bind(&namespace).unwrap();
    let notifications = server.disconnect_notifications();
    // Fill with accepted requests whose operations have not yet reached GPUI.
    // Departure frees tasks, but must not remove/cancel their queued operations.
    for index in 0..CAPACITY {
        let mut client = PipeClient::connect(&namespace, stage()).await.unwrap();
        send_raw(&mut client, &format!("queued-{index}")).await;
        wait_until(|| server.incoming.as_ref().unwrap().len() == index + 1).await;
        drop(client);
        wait_until(|| server.active.load(Ordering::SeqCst) == 0).await;
    }
    let mut rejected = PipeClient::connect(&namespace, stage()).await.unwrap();
    send_raw(&mut rejected, "rejected").await;
    assert!(io::read_frame(&mut rejected.pipe, stage()).await.is_err());
    wait_until(|| server.active.load(Ordering::SeqCst) == 0).await;
    assert_eq!(server.incoming.as_ref().unwrap().len(), CAPACITY);
    timeout(Duration::from_secs(1), notifications.notified())
        .await
        .unwrap();
    for index in 0..CAPACITY {
        let incoming = receive(&mut server).await;
        assert_eq!(incoming.request.id.0, format!("queued-{index}"));
        assert!(!incoming.reply.is_alive());
    }
    assert!(server.incoming.as_mut().unwrap().try_recv().is_err());
    let mut next = PipeClient::connect(&namespace, stage()).await.unwrap();
    send_raw(&mut next, "next").await;
    assert_eq!(receive(&mut server).await.request.id.0, "next");
}

#[tokio::test(flavor = "current_thread")]
async fn connection_limit_rejects_without_spawning_waiters_and_shutdown_reaps_all() {
    let namespace = Namespace::isolated();
    let mut server = PipeServer::bind(&namespace).unwrap();
    let mut clients = Vec::new();
    for _ in 0..CAPACITY {
        clients.push(PipeClient::connect(&namespace, stage()).await.unwrap());
    }
    wait_until(|| server.active.load(Ordering::SeqCst) == CAPACITY).await;
    let mut excess = PipeClient::connect(&namespace, stage()).await.unwrap();
    assert!(io::read_frame(&mut excess.pipe, stage()).await.is_err());
    assert_eq!(server.active.load(Ordering::SeqCst), CAPACITY);
    server.shutdown();
    assert_eq!(server.active.load(Ordering::SeqCst), 0);
    drop(clients);
}

#[tokio::test(flavor = "current_thread")]
async fn completion_disconnect_race_never_leaves_a_reply_or_connection_task() {
    let namespace = Namespace::isolated();
    let mut server = PipeServer::bind(&namespace).unwrap();
    for iteration in 0..32 {
        let mut client = PipeClient::connect(&namespace, stage()).await.unwrap();
        send_raw(&mut client, "colliding-external-id").await;
        let incoming = receive(&mut server).await;
        if iteration % 2 == 0 {
            drop(client);
            wait_until(|| !incoming.reply.is_alive()).await;
            assert!(respond(incoming, Ok(())).is_err());
        } else {
            // A completion and peer departure can be ready on the same runtime
            // turn. Either send result is legal; neither may retain a worker.
            let _ = respond(incoming, Ok(()));
            drop(client);
        }
        wait_until(|| server.active.load(Ordering::SeqCst) == 0).await;
    }
}

#[test]
fn bind_reports_first_pipe_failure_synchronously() {
    let namespace = Namespace::isolated();
    let _server = PipeServer::bind(&namespace).unwrap();
    assert!(PipeServer::bind(&namespace).is_err());
}
