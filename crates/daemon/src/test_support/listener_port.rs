//! Keep fixture listeners separate from outbound clients and other fixture processes.
use std::fs::{File, OpenOptions};
use std::io;
use std::net::{Ipv4Addr, SocketAddr, TcpListener};
use std::path::{Path, PathBuf};

pub(crate) struct ListenerPortLease {
    pub(crate) address: SocketAddr,
    _lease: File,
}

impl ListenerPortLease {
    pub(crate) fn reserve() -> io::Result<Self> {
        let directory = lease_directory()?;
        let (first, last) = ephemeral_range()?;
        let candidates: Vec<_> = (10000..30000)
            .filter(|port| !(first..=last).contains(port))
            .collect();
        let mut random = [0; 2];
        getrandom::fill(&mut random)
            .map_err(|_| io::Error::other("cannot randomize fixture ports"))?;
        let start = usize::from(u16::from_ne_bytes(random));
        Self::reserve_from(
            &directory,
            candidates
                .iter()
                .cycle()
                .skip(start)
                .take(candidates.len().min(128))
                .copied(),
        )
    }

    fn reserve_from(directory: &Path, ports: impl IntoIterator<Item = u16>) -> io::Result<Self> {
        for port in ports {
            let mut options = OpenOptions::new();
            options.create(true).truncate(false).read(true).write(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600).custom_flags(nix::libc::O_NOFOLLOW);
            }
            let lease = options.open(directory.join(format!("{port}.lock")))?;
            match fs2::FileExt::try_lock_exclusive(&lease) {
                Ok(()) => {}
                Err(error)
                    if error.raw_os_error() == fs2::lock_contended_error().raw_os_error() =>
                {
                    continue;
                }
                Err(error) => return Err(error),
            }
            let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
            match TcpListener::bind(address) {
                Ok(probe) => drop(probe),
                Err(error) if error.kind() == io::ErrorKind::AddrInUse => continue,
                Err(error) => return Err(error),
            }
            // The real Gateway binds the socket. Keep the file lease until it has stopped;
            // never unlink lock files, since another process may already have opened one.
            return Ok(Self {
                address,
                _lease: lease,
            });
        }
        Err(io::Error::new(
            io::ErrorKind::AddrNotAvailable,
            "no unoccupied fixture listener address",
        ))
    }
}

fn lease_directory() -> io::Result<PathBuf> {
    // Match publication_product.py on Unix, across worktrees and private runner TMPDIRs.
    #[cfg(unix)]
    let directory = PathBuf::from(format!(
        "/tmp/hiroute-product-listener-ports-{}",
        nix::unistd::getuid()
    ));
    #[cfg(windows)]
    let directory = PathBuf::from(
        std::env::var_os("LOCALAPPDATA")
            .ok_or_else(|| io::Error::other("LOCALAPPDATA is unavailable"))?,
    )
    .join("hiroute-product-listener-ports");
    let builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    let builder = {
        use std::os::unix::fs::DirBuilderExt;
        let mut builder = builder;
        builder.mode(0o700);
        builder
    };
    match builder.create(&directory) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error),
    }
    let metadata = std::fs::symlink_metadata(&directory)?;
    let valid = metadata.is_dir() && !metadata.file_type().is_symlink();
    #[cfg(unix)]
    let valid = {
        use std::os::unix::fs::MetadataExt;
        valid
            && metadata.uid() == nix::unistd::getuid().as_raw()
            && metadata.mode() & 0o777 == 0o700
    };
    if !valid {
        return Err(io::Error::other(
            "fixture listener lease directory is not private and owned",
        ));
    }
    Ok(directory)
}

#[cfg(any(target_os = "linux", target_os = "macos", windows))]
fn ephemeral_range() -> io::Result<(u16, u16)> {
    #[cfg(target_os = "linux")]
    let bounds = std::fs::read_to_string("/proc/sys/net/ipv4/ip_local_port_range")?;
    #[cfg(target_os = "macos")]
    let bounds = {
        let output = std::process::Command::new("/usr/sbin/sysctl")
            .args([
                "-n",
                "net.inet.ip.portrange.first",
                "net.inet.ip.portrange.last",
            ])
            .output()?;
        if !output.status.success() {
            return Err(io::Error::other(
                "cannot read outbound ephemeral port range",
            ));
        }
        String::from_utf8_lossy(&output.stdout).into_owned()
    };
    #[cfg(windows)]
    let bounds = {
        let executable =
            hiroute_platform::windows::launcher::system_executable(Path::new("netsh.exe"))?;
        let output = std::process::Command::new(executable)
            .args(["int", "ipv4", "show", "dynamicport", "tcp"])
            .output()?;
        let numbers: Vec<u32> = String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter_map(|line| line.split_once(':')?.1.trim().parse().ok())
            .collect();
        if !output.status.success() || numbers.len() != 2 || numbers[1] == 0 {
            return Err(io::Error::other(
                "cannot read outbound ephemeral port range",
            ));
        }
        format!("{} {}", numbers[0], numbers[0] + numbers[1] - 1)
    };
    let ports: Vec<u16> = bounds
        .split_whitespace()
        .map(str::parse)
        .collect::<Result<_, _>>()
        .map_err(io::Error::other)?;
    match ports.as_slice() {
        [first, last] if first <= last => Ok((*first, *last)),
        _ => Err(io::Error::other("invalid outbound ephemeral port range")),
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn ephemeral_range() -> io::Result<(u16, u16)> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "unknown outbound ephemeral port range",
    ))
}

#[test]
fn listener_lease_avoids_outbound_ephemeral_ports() {
    let lease = ListenerPortLease::reserve().unwrap();
    let (first, last) = ephemeral_range().unwrap();
    assert!(!(first..=last).contains(&lease.address.port()));
}

#[test]
fn listener_lease_survives_socket_handoff_and_releases_on_drop() {
    const TEST: &str =
        "test_support::listener_port::listener_lease_survives_socket_handoff_and_releases_on_drop";
    const CHILD_PORT: &str = "HIROUTE_TEST_LEASED_LISTENER_PORT";
    if let Ok(port) = std::env::var(CHILD_PORT) {
        assert_eq!(
            ListenerPortLease::reserve_from(&lease_directory().unwrap(), [port.parse().unwrap()])
                .err()
                .unwrap()
                .kind(),
            io::ErrorKind::AddrNotAvailable
        );
        return;
    }
    let global = ListenerPortLease::reserve().unwrap();
    let address = global.address;
    let child_tmp = super::private_tempdir();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", TEST, "--nocapture"])
        .env(CHILD_PORT, address.port().to_string())
        .env("TMPDIR", child_tmp.path())
        .env("TMP", child_tmp.path())
        .env("TEMP", child_tmp.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));

    // Keep the global lease while exercising release in a private lock namespace,
    // so another fixture cannot take this address during the release assertion.
    let directory = super::private_tempdir();
    let lease = ListenerPortLease::reserve_from(directory.path(), [address.port()]).unwrap();
    let gateway_socket = TcpListener::bind(address).unwrap();
    drop(gateway_socket);
    assert_eq!(
        ListenerPortLease::reserve_from(directory.path(), [address.port()])
            .err()
            .unwrap()
            .kind(),
        io::ErrorKind::AddrNotAvailable
    );
    drop(lease);
    assert_eq!(
        ListenerPortLease::reserve_from(directory.path(), [address.port()])
            .unwrap()
            .address,
        address
    );
}
