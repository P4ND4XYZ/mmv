use super::*;
use crate::app::{ExternalOperation, ExternalRequestId, ScrollDirection};
use std::time::Duration;
use tokio::{io::AsyncWriteExt, time::timeout};

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
            tokio::task::yield_now().await;
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
fn assert_closed_error(error: stdio::Error) {
    assert!(
        matches!(
            error.kind(),
            stdio::ErrorKind::UnexpectedEof
                | stdio::ErrorKind::BrokenPipe
                | stdio::ErrorKind::ConnectionReset
        ),
        "expected peer closure, got {error:?}"
    );
}
async fn assert_closed(client: &mut PipeClient) {
    assert_closed_error(
        io::read_exact(&mut client.pipe, &mut [0], stage())
            .await
            .unwrap_err(),
    );
}
async fn peer_disconnected(reply: &Reply, notifications: &Notify) {
    timeout(Duration::from_secs(3), async {
        // Notifications coalesce and may include earlier peers. Liveness, not
        // the number of permits, identifies this particular departure.
        while reply.is_alive() {
            notifications.notified().await;
        }
    })
    .await
    .expect("peer did not disconnect before test deadline");
}
async fn round_trip(server: &mut PipeServer, namespace: &Namespace, id: &str) {
    let client = PipeClient::connect(namespace, stage()).await.unwrap();
    let expected = request(id);
    let worker = tokio::spawn(client.request(expected.clone(), stage()));
    let incoming = receive(server).await;
    assert_eq!(incoming.request, expected);
    respond(incoming, Ok(())).unwrap();
    assert_eq!(
        worker.await.unwrap().unwrap(),
        Response {
            id: expected.id,
            outcome: Ok(())
        }
    );
}

fn absolute_path(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(name)
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
// A pending future models the slow Windows connect without relying on the OS's
// refusal delay. Its lifetime also proves that cancellation releases the attempt.
struct PendingConnect<'a>(&'a std::cell::Cell<usize>);
impl Drop for PendingConnect<'_> {
    fn drop(&mut self) {
        self.0.set(self.0.get() + 1);
    }
}

#[tokio::test(flavor = "current_thread")]
async fn pending_connection_attempt_times_out_and_drops_its_future() {
    let dropped = std::cell::Cell::new(0);
    let attempt = PendingConnect(&dropped);
    let result = timeout(
        Duration::from_millis(400),
        connect_attempt(async move {
            let _attempt = attempt;
            std::future::pending::<stdio::Result<()>>().await
        }),
    )
    .await
    .expect("one connect attempt must finish before the stage deadline");
    let error = result.unwrap_err();
    assert_eq!(error.kind(), stdio::ErrorKind::TimedOut);
    assert!(
        retryable(&error),
        "only this attempt's timeout is retryable"
    );
    assert_eq!(dropped.get(), 1);
}

#[tokio::test(flavor = "current_thread")]
async fn pending_connection_attempt_is_dropped_before_retry_and_success() {
    let attempts = std::cell::Cell::new(0);
    let dropped = std::cell::Cell::new(0);
    let result = timeout(
        Duration::from_secs(1),
        connect_with(Deadline::after(Duration::from_millis(700)), || {
            let index = attempts.get();
            attempts.set(index + 1);
            let dropped = &dropped;
            async move {
                if index == 0 {
                    let _attempt = PendingConnect(dropped);
                    std::future::pending::<()>().await;
                }
                assert_eq!(dropped.get(), 1);
                Ok(42)
            }
        }),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(result, 42);
    assert_eq!(attempts.get(), 2);
    assert_eq!(dropped.get(), 1);
}

#[tokio::test(flavor = "current_thread")]
async fn pending_connection_retries_preserve_the_original_stage_deadline() {
    for duration in [Duration::from_millis(35), Duration::from_millis(275)] {
        let attempts = std::cell::Cell::new(0);
        let dropped = std::cell::Cell::new(0);
        let deadline = Deadline::after(duration);
        let error = timeout(
            duration + Duration::from_millis(50),
            connect_with(deadline, || {
                attempts.set(attempts.get() + 1);
                let dropped = &dropped;
                async move {
                    let _attempt = PendingConnect(dropped);
                    std::future::pending::<stdio::Result<()>>().await
                }
            }),
        )
        .await
        .expect("retries must not extend the original stage deadline")
        .unwrap_err();
        assert!(deadline.expired(Instant::now()));
        assert_eq!(error.kind(), stdio::ErrorKind::TimedOut);
        assert!(!retryable(&error), "stage expiry is terminal");
        assert_eq!(dropped.get(), attempts.get());
        if duration < Duration::from_millis(100) {
            assert_eq!(attempts.get(), 1, "respect the shorter stage remainder");
        } else {
            assert!(attempts.get() > 1, "attempt timeouts must allow retry");
        }
    }
}

#[tokio::test(flavor = "current_thread")]
async fn connection_non_retryable_errors_fail_immediately() {
    for kind in [
        stdio::ErrorKind::PermissionDenied,
        stdio::ErrorKind::InvalidInput,
        stdio::ErrorKind::TimedOut,
    ] {
        let attempts = std::cell::Cell::new(0);
        let error = connect_with(stage(), || {
            attempts.set(attempts.get() + 1);
            std::future::ready(Err::<(), _>(stdio::Error::new(kind, "connector error")))
        })
        .await
        .unwrap_err();
        assert_eq!(error.kind(), kind);
        assert_eq!(attempts.get(), 1);
        assert!(!retryable(&error));
    }
}

#[tokio::test(flavor = "current_thread")]
async fn real_tcp_single_attempt_to_closed_port_is_bounded_and_retryable() {
    let namespace = Namespace::isolated();
    let error = timeout(
        Duration::from_millis(400),
        PipeClient::try_connect(&namespace),
    )
    .await
    .expect("single attempt must not wait for the OS refusal delay")
    .err()
    .expect("isolated port has no listener");
    assert!(retryable(&error));
}

#[tokio::test(flavor = "current_thread")]
async fn real_tcp_connect_retries_until_delayed_listener_is_ready() {
    let namespace = Namespace::isolated();
    timeout(Duration::from_secs(3), async {
        let connect = PipeClient::connect(&namespace, stage());
        let listen = async {
            tokio::time::sleep(Duration::from_millis(140)).await;
            let listener = TcpListener::bind(namespace.address()).await.unwrap();
            let (stream, _) = listener.accept().await.unwrap();
            (listener, stream)
        };
        let (client, (_listener, _stream)) = tokio::join!(connect, listen);
        assert!(client.is_ok());
    })
    .await
    .expect("client did not connect after delayed listener readiness");
}

#[test]
fn protocol_round_trips_every_public_operation_and_failure_reason() {
    let mut operations = vec![
        ExternalOperation::Open {
            path: absolute_path("日本語 space/memo.md"),
            line: None,
        },
        ExternalOperation::Top,
        ExternalOperation::Bottom,
        ExternalOperation::Back,
        ExternalOperation::Forward,
    ];
    operations.push(ExternalOperation::Open {
        path: absolute_path("日本語 space/memo.md"),
        line: std::num::NonZeroU64::new(u64::MAX),
    });
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
    assert_eq!(&encoded[..], &[9, 0, 0, 0, 3, 0, 1, 1, 0, 0, 0, b'a', 7]);
    for bytes in [
        vec![],
        vec![1, 0, 1],
        vec![3, 0, 3],
        vec![3, 0, 1, 255, 255, 255, 255],
        vec![3, 0, 1, 1, 0, 0, 0, 0xff, 7],
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
            operation: ExternalOperation::Open {
                path: "relative.md".into(),
                line: None
            }
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
    assert!(Response::decode(&[3, 0, 2, 1, 0, 0, 0, b'a', 2]).is_err());
}

#[test]
fn open_rejects_invalid_line_flags_zero_truncation_extra_data_and_old_versions() {
    let original = Request {
        id: ExternalRequestId("line".into()),
        operation: ExternalOperation::Open {
            path: absolute_path("memo.md"),
            line: std::num::NonZeroU64::new(42),
        },
    }
    .encode()
    .unwrap();
    let payload = &original[4..];
    for flag in [2, 255] {
        let mut bytes = payload.to_vec();
        let ix = bytes.len() - 9;
        bytes[ix] = flag;
        assert!(Request::decode(&bytes).is_err());
    }
    let mut zero = payload.to_vec();
    let ix = zero.len() - 8;
    zero[ix..].fill(0);
    assert!(Request::decode(&zero).is_err());
    for missing in 1..=8 {
        assert!(Request::decode(&payload[..payload.len() - missing]).is_err());
    }
    let mut extra = payload.to_vec();
    extra.push(0);
    assert!(Request::decode(&extra).is_err());
    let mut old = payload.to_vec();
    old[0] = 1;
    assert!(Request::decode(&old).is_err());
}

#[tokio::test(flavor = "current_thread")]
async fn real_tcp_parallel_requests_receive_reverse_order_results_with_original_ids() {
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
    drop(client);
    peer_disconnected(&incoming.reply, &server.disconnect_notifications()).await;
    assert_eq!(
        respond(incoming, Ok(())).unwrap_err().kind(),
        stdio::ErrorKind::BrokenPipe
    );
    let worker = tokio::spawn(silent.request(request("formerly-silent"), stage()));
    let incoming = receive(&mut server).await;
    assert_eq!(incoming.request.id.0, "formerly-silent");
    respond(incoming, Ok(())).unwrap();
    assert_eq!(worker.await.unwrap().unwrap().outcome, Ok(()));
}

// Start serve only after a complete request (and its invalid tail) is readable.
// This fixes the ordering rather than depending on the accept/runtime race.
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
    assert!(!incoming.reply.is_alive());
    assert_eq!(
        respond(incoming, Ok(())).unwrap_err().kind(),
        stdio::ErrorKind::BrokenPipe
    );
    assert_closed(&mut client).await;
}

#[tokio::test(flavor = "current_thread")]
async fn real_tcp_rejects_bad_boundaries_and_truncated_requests_without_dispatch() {
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
            assert_closed(&mut client).await;
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
    peer_disconnected(&incoming.reply, &server.disconnect_notifications()).await;
    assert_eq!(
        respond(incoming, Ok(())).unwrap_err().kind(),
        stdio::ErrorKind::BrokenPipe
    );
    assert!(server.incoming.as_mut().unwrap().try_recv().is_err());
    round_trip(&mut server, &namespace, "after-timeout").await;
}

#[tokio::test(flavor = "current_thread")]
async fn shutdown_cancels_pending_accept_reads_and_app_response_waits_and_joins() {
    let namespace = Namespace::isolated();
    let mut server = PipeServer::bind(&namespace).unwrap();
    let mut silent = PipeClient::connect(&namespace, stage()).await.unwrap();
    let mut client = PipeClient::connect(&namespace, stage()).await.unwrap();
    send_raw(&mut client, "waiting").await;
    let incoming = receive(&mut server).await;
    let start = Instant::now();
    server.shutdown();
    server.shutdown();
    assert!(start.elapsed() < Duration::from_secs(2));
    assert!(!incoming.reply.is_alive());
    assert_eq!(
        respond(incoming, Ok(())).unwrap_err().kind(),
        stdio::ErrorKind::BrokenPipe
    );
    assert_closed(&mut silent).await;
    assert_closed(&mut client).await;
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
    assert_closed_error(worker.await.unwrap().unwrap_err());
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
        // Consume each departure before creating another peer; the final one
        // must wake us even while the request queue is completely full.
        timeout(Duration::from_secs(3), notifications.notified())
            .await
            .unwrap();
    }
    let mut rejected = PipeClient::connect(&namespace, stage()).await.unwrap();
    send_raw(&mut rejected, "rejected").await;
    assert_closed(&mut rejected).await;
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
    round_trip(&mut server, &namespace, "next").await;
}

#[tokio::test(flavor = "current_thread")]
async fn connection_limit_rejects_overflow_and_reuses_disconnected_capacity() {
    let namespace = Namespace::isolated();
    let mut server = PipeServer::bind(&namespace).unwrap();
    let notifications = server.disconnect_notifications();
    let mut clients = Vec::new();
    let mut accepted = Vec::new();
    for index in 0..CAPACITY {
        let mut client = PipeClient::connect(&namespace, stage()).await.unwrap();
        let id = format!("live-{index}");
        send_raw(&mut client, &id).await;
        let incoming = receive(&mut server).await;
        assert_eq!(incoming.request.id.0, id);
        clients.push(client);
        accepted.push(incoming);
    }
    assert!(accepted.iter().all(|incoming| incoming.reply.is_alive()));
    let mut excess = PipeClient::connect(&namespace, stage()).await.unwrap();
    // Reject before the 5-second request-read timeout; an admitted silent
    // connection timing out must not be mistaken for capacity rejection.
    timeout(Duration::from_secs(2), assert_closed(&mut excess))
        .await
        .expect("overflow connection was not promptly rejected");
    assert!(server.incoming.as_mut().unwrap().try_recv().is_err());

    drop(clients.pop().unwrap());
    let departed = accepted.pop().unwrap();
    peer_disconnected(&departed.reply, &notifications).await;
    assert_eq!(
        respond(departed, Ok(())).unwrap_err().kind(),
        stdio::ErrorKind::BrokenPipe
    );

    timeout(Duration::from_secs(3), async {
        loop {
            // Departure is published before JoinSet reaping. A connection may
            // still be rejected in that interval; only dispatch AND a complete
            // response prove that the released capacity is reusable.
            let client = PipeClient::connect(&namespace, stage()).await.unwrap();
            let response = client.request(request("replacement"), stage());
            tokio::pin!(response);
            tokio::select! {
                incoming = receive(&mut server) => {
                    assert_eq!(incoming.request.id.0, "replacement");
                    respond(incoming, Ok(())).unwrap();
                    let response = response.await.unwrap();
                    assert_eq!(response.id.0, "replacement");
                    assert_eq!(response.outcome, Ok(()));
                    break;
                }
                result = &mut response => {
                    assert_closed_error(result.unwrap_err());
                }
            }
        }
    })
    .await
    .expect("released connection capacity was not reusable");
    assert!(accepted.iter().all(|incoming| incoming.reply.is_alive()));
    server.shutdown();
    for incoming in accepted {
        assert!(!incoming.reply.is_alive());
        assert_eq!(
            respond(incoming, Ok(())).unwrap_err().kind(),
            stdio::ErrorKind::BrokenPipe
        );
    }
    for client in &mut clients {
        assert_closed(client).await;
    }
}

#[tokio::test(flavor = "current_thread")]
async fn completion_disconnect_race_notifies_and_keeps_accepting_requests() {
    let namespace = Namespace::isolated();
    let mut server = PipeServer::bind(&namespace).unwrap();
    let notifications = server.disconnect_notifications();
    for iteration in 0..CAPACITY * 2 {
        let mut client = PipeClient::connect(&namespace, stage()).await.unwrap();
        send_raw(&mut client, "colliding-external-id").await;
        let incoming = receive(&mut server).await;
        if iteration % 2 == 0 {
            drop(client);
            timeout(Duration::from_secs(3), notifications.notified())
                .await
                .unwrap();
            assert!(!incoming.reply.is_alive());
            assert_eq!(
                respond(incoming, Ok(())).unwrap_err().kind(),
                stdio::ErrorKind::BrokenPipe
            );
        } else {
            // Either completion or departure may win. Consume exactly this
            // peer's notification before the next iteration, avoiding stale
            // permits and coalescing across different connections.
            let _ = respond(incoming, Ok(()));
            drop(client);
            timeout(Duration::from_secs(3), notifications.notified())
                .await
                .unwrap();
        }
    }
    round_trip(&mut server, &namespace, "after-race").await;
}

#[test]
fn bind_reports_port_conflict_synchronously() {
    let namespace = Namespace::isolated();
    let _server = PipeServer::bind(&namespace).unwrap();
    assert!(PipeServer::bind(&namespace).is_err());
}

#[test]
fn native_paths_are_lossless_and_other_representations_are_rejected() {
    #[cfg(windows)]
    let path = {
        use std::os::windows::ffi::OsStringExt;
        let mut units: Vec<_> = r"C:\native-".encode_utf16().collect();
        units.extend([0xd800, b'.' as u16, b'm' as u16, b'd' as u16]);
        std::path::PathBuf::from(std::ffi::OsString::from_wide(&units))
    };
    #[cfg(target_os = "linux")]
    let path = {
        use std::os::unix::ffi::OsStringExt;
        std::path::PathBuf::from(std::ffi::OsString::from_vec(
            b"/tmp/native-\xff.md".to_vec(),
        ))
    };
    let original = Request {
        id: ExternalRequestId("native".into()),
        operation: ExternalOperation::Open { path, line: None },
    };
    let encoded = original.encode().unwrap();
    assert_eq!(Request::decode(&encoded[4..]).unwrap(), original);
    let encoding_index = 3 + 4 + "native".len() + 1;
    for encoding in [0, 255, if cfg!(windows) { 2 } else { 1 }] {
        let mut payload = encoded[4..].to_vec();
        payload[encoding_index] = encoding;
        assert!(Request::decode(&payload).is_err());
    }
    #[cfg(windows)]
    {
        let mut payload = encoded[4..].to_vec();
        let length = encoding_index + 1;
        payload[length..length + 4].copy_from_slice(&1u32.to_le_bytes());
        assert!(Request::decode(&payload).is_err());
    }
}

async fn pair() -> (TcpStream, PipeClient) {
    let n = Namespace::isolated();
    let listener = TcpListener::bind(n.address()).await.unwrap();
    let client = PipeClient::try_connect(&n).await.unwrap();
    let (server, _) = listener.accept().await.unwrap();
    (server, client)
}

#[tokio::test(flavor = "current_thread")]
async fn readable_extra_byte_is_rejected_before_dispatch() {
    let (pipe, mut client) = pair().await;
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
    assert!(incoming.try_recv().is_err());
    assert!(!peer.alive.load(Ordering::Acquire));
}

#[tokio::test(flavor = "current_thread")]
async fn disconnected_before_dispatch_is_rejected() {
    let (pipe, mut client) = pair().await;
    send_raw(&mut client, "must-not-dispatch").await;
    client.pipe.shutdown().await.unwrap();
    stage().until(pipe.readable()).await.unwrap();
    let (sender, mut incoming) = mpsc::channel(CAPACITY);
    let peer = Arc::new(Peer {
        id: ConnectionId(2),
        alive: AtomicBool::new(true),
        disconnected: Arc::new(Notify::new()),
    });
    let result = stage().until(serve(pipe, sender, peer.clone())).await;
    assert!(result.is_err());
    assert!(incoming.try_recv().is_err());
    assert!(!peer.alive.load(Ordering::Acquire));
}

#[tokio::test(flavor = "current_thread")]
async fn mismatched_truncated_or_old_response_fails_without_resend() {
    for mode in 0..3 {
        let n = Namespace::isolated();
        let listener = TcpListener::bind(n.address()).await.unwrap();
        let worker = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            io::read_frame(&mut stream, stage()).await.unwrap();
            let mut bytes = Response {
                id: ExternalRequestId(if mode == 0 { "wrong" } else { "expected" }.into()),
                outcome: Ok(()),
            }
            .encode()
            .unwrap();
            if mode == 1 {
                bytes.truncate(6);
            }
            if mode == 2 {
                bytes[4] = 2;
            }
            io::write_all(&mut stream, &bytes, stage()).await.unwrap();
            stream.shutdown().await.unwrap();
            // One request per connection, and no second connection/resend.
            assert!(
                timeout(Duration::from_millis(120), listener.accept())
                    .await
                    .is_err()
            );
        });
        let error = PipeClient::connect(&n, stage())
            .await
            .unwrap()
            .request(request("expected"), stage())
            .await
            .unwrap_err();
        assert_eq!(
            error.kind(),
            if mode == 1 {
                stdio::ErrorKind::UnexpectedEof
            } else {
                stdio::ErrorKind::InvalidData
            }
        );
        worker.await.unwrap();
    }
}

#[tokio::test(flavor = "current_thread")]
async fn partial_frame_timeout_and_unreading_peer_do_not_block_teardown() {
    let (mut stream, mut client) = pair().await;
    client.pipe.write_all(&[9, 0]).await.unwrap();
    assert_eq!(
        io::read_frame(&mut stream, short())
            .await
            .unwrap_err()
            .kind(),
        stdio::ErrorKind::TimedOut
    );
    drop(stream);
    drop(client);
    let (mut stream, _client) = pair().await;
    let bytes = vec![0x42; 65536];
    let deadline = short();
    let error = loop {
        if let Err(error) = io::write_all(&mut stream, &bytes, deadline).await {
            break error;
        }
    };
    assert_eq!(error.kind(), stdio::ErrorKind::TimedOut);
    let start = Instant::now();
    drop(stream);
    assert!(start.elapsed() < Duration::from_secs(1));
}

#[tokio::test(flavor = "current_thread")]
async fn complete_response_needs_no_receipt_or_server_disconnect() {
    let n = Namespace::isolated();
    let listener = TcpListener::bind(n.address()).await.unwrap();
    let worker = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        io::read_frame(&mut stream, stage()).await.unwrap();
        let bytes = Response {
            id: ExternalRequestId("no-receipt".into()),
            outcome: Ok(()),
        }
        .encode()
        .unwrap();
        io::write_all(&mut stream, &bytes, stage()).await.unwrap();
        // Keep stream open until the CLI has returned and closed it.
        assert_eq!(
            io::read_exact(&mut stream, &mut [0], stage())
                .await
                .unwrap_err()
                .kind(),
            stdio::ErrorKind::UnexpectedEof
        );
    });
    let response = PipeClient::connect(&n, stage())
        .await
        .unwrap()
        .request(request("no-receipt"), short())
        .await
        .unwrap();
    assert_eq!(response.outcome, Ok(()));
    worker.await.unwrap();
}
