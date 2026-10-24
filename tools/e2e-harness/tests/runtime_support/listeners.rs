use std::io;
use std::net::{SocketAddr, TcpListener};

/// Keep both ports occupied while preparing one fixture, so the OS cannot
/// recycle its control port as the product port before the child is launched.
pub(super) struct ListenerReservations {
    product: TcpListener,
    control: Option<TcpListener>,
}

impl ListenerReservations {
    pub(super) fn bind(with_control: bool) -> io::Result<Self> {
        let product = TcpListener::bind("127.0.0.1:0")?;
        let control = with_control
            .then(|| TcpListener::bind("127.0.0.1:0"))
            .transpose()?;
        Ok(Self { product, control })
    }

    pub(super) fn product_address(&self) -> SocketAddr {
        self.product.local_addr().unwrap()
    }

    pub(super) fn control_address(&self) -> Option<SocketAddr> {
        self.control
            .as_ref()
            .map(|listener| listener.local_addr().unwrap())
    }
}

#[test]
fn product_and_control_ports_stay_distinct_and_reserved_until_launch() {
    let reservations = ListenerReservations::bind(true).unwrap();
    let product = reservations.product_address();
    let control = reservations.control_address().unwrap();
    assert_ne!(product, control);
    for address in [product, control] {
        assert!(address.ip().is_loopback());
        assert_ne!(address.port(), 0);
        assert_eq!(
            TcpListener::bind(address).unwrap_err().kind(),
            io::ErrorKind::AddrInUse,
            "fixture preparation must keep both addresses reserved"
        );
    }
    drop(reservations);
    let product = TcpListener::bind(product).expect("release product port for the child");
    let control = TcpListener::bind(control).expect("release control port for the child");
    assert_ne!(product.local_addr().unwrap(), control.local_addr().unwrap());
}

#[test]
fn an_uncontrolled_fixture_reserves_only_its_product_listener() {
    let reservations = ListenerReservations::bind(false).unwrap();
    assert!(reservations.control_address().is_none());
    assert_eq!(
        TcpListener::bind(reservations.product_address())
            .unwrap_err()
            .kind(),
        io::ErrorKind::AddrInUse
    );
}
