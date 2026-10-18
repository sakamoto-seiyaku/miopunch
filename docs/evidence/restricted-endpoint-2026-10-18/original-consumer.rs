use iroh::endpoint::{Accepting, AfterHandshakeOutcome, IncomingZeroRttConnection};

fn incoming(accepting: Accepting) -> IncomingZeroRttConnection {
    accepting.into_0rtt()
}

fn outcome(result: AfterHandshakeOutcome) -> bool {
    match result {
        AfterHandshakeOutcome::Accept => true,
        AfterHandshakeOutcome::Reject { .. } => false,
    }
}

fn main() {
    let _ = incoming;
    assert!(outcome(AfterHandshakeOutcome::Accept));
}
