//! Native IO only. Framing, authentication and application dispatch are shared in ipc.rs.
#[cfg(unix)]
mod unix;
#[cfg(unix)]
pub(super) use unix::{Listener, Stream};
#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub(super) use windows::{Listener, Stream};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::Profile;
    use std::io::{Read, Write};
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    #[test]
    fn private_native_transport_roundtrip_and_peer_identity() {
        let temporary = tempfile::tempdir().unwrap();
        let profile = Profile::new(temporary.path().join("profile")).unwrap();
        let server_profile = profile.clone();
        let (ready_sender, ready_receiver) = mpsc::sync_channel(1);
        let (bound_sender, bound_receiver) = mpsc::sync_channel(1);
        let worker = std::thread::spawn(move || {
            // A Windows pending OVERLAPPED listener must stay on its owning thread.
            let mut listener = Listener::bind(&server_profile).unwrap();
            bound_sender.send(()).unwrap();
            let deadline = Instant::now() + Duration::from_secs(5);
            let mut stream = loop {
                match listener.accept() {
                    Ok(stream) => break stream,
                    Err(e)
                        if e.kind() == std::io::ErrorKind::WouldBlock
                            && Instant::now() < deadline =>
                    {
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Err(e) => panic!("Native accept failed: {e}"),
                }
            };
            stream.verify_peer().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut input = [0u8; 4];
            stream.read_exact(&mut input).unwrap();
            assert_eq!(&input, b"ping");
            stream.write_all(b"pong").unwrap();
            // Keep the stream alive until the client has consumed the response.
            ready_receiver.recv_timeout(Duration::from_secs(3)).unwrap();
        });
        bound_receiver.recv_timeout(Duration::from_secs(3)).unwrap();
        let mut client = Stream::connect(&profile).unwrap();
        client.verify_peer().unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        client.write_all(b"ping").unwrap();
        let mut response = [0u8; 4];
        client.read_exact(&mut response).unwrap();
        assert_eq!(&response, b"pong");
        ready_sender.send(()).unwrap();
        worker.join().unwrap();
        assert!(!profile.database().exists());
    }

    #[cfg(unix)]
    #[test]
    fn accepted_stream_blocks_until_late_client_bytes_arrive() {
        let temporary = tempfile::tempdir().unwrap();
        let profile = Profile::new(temporary.path().join("profile")).unwrap();
        let mut listener = Listener::bind(&profile).unwrap();
        let mut client = Stream::connect(&profile).unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut server = loop {
            match listener.accept() {
                Ok(stream) => break stream,
                Err(e)
                    if e.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < deadline =>
                {
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(e) => panic!("Native accept failed: {e}"),
            }
        };
        // The polling listener is non-blocking; the accepted stream must honor its timeout.
        server
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let writer = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            client.write_all(b"late").unwrap();
            client
        });
        let mut input = [0u8; 4];
        server.read_exact(&mut input).unwrap();
        assert_eq!(&input, b"late");
        drop(writer.join().unwrap());
    }

    #[test]
    fn operating_system_lease_prevents_a_second_writer_and_can_be_reacquired() {
        let temporary = tempfile::tempdir().unwrap();
        let profile = Profile::new(temporary.path().join("profile")).unwrap();
        let lease = profile.acquire().unwrap();
        assert_eq!(profile.acquire().unwrap_err().code, "CORE_UNAVAILABLE");
        drop(lease);
        let _replacement = profile.acquire().unwrap();
        assert!(profile.directory().join("core.lock").exists());
    }

    #[test]
    fn desktop_lease_is_exclusive_but_independent_of_the_core_lease() {
        let temporary = tempfile::tempdir().unwrap();
        let profile = Profile::new(temporary.path().join("profile")).unwrap();
        let desktop = profile.desktop_lease().unwrap();
        assert_eq!(profile.desktop_lease().unwrap_err().code, "DESKTOP_RUNNING");
        // A running desktop never holds or implies the Core writer lease.
        let core = profile.acquire().unwrap();
        assert_eq!(profile.acquire().unwrap_err().code, "CORE_UNAVAILABLE");
        drop(core);
        // A second launch leaves one marker; the running desktop consumes it exactly once.
        assert!(!profile.take_desktop_activation().unwrap());
        profile.request_desktop_activation().unwrap();
        profile.request_desktop_activation().unwrap();
        assert!(profile.take_desktop_activation().unwrap());
        assert!(!profile.take_desktop_activation().unwrap());
        drop(desktop);
        let _again = profile.desktop_lease().unwrap();
        assert!(!profile.database().exists());
    }

    #[cfg(windows)]
    #[test]
    fn windows_pipe_name_cannot_be_rebound_while_listener_is_alive() {
        let temporary = tempfile::tempdir().unwrap();
        let profile = Profile::new(temporary.path().join("profile")).unwrap();
        let _listener = Listener::bind(&profile).unwrap();
        assert!(Listener::bind(&profile).is_err());
    }
}
