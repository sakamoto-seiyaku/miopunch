use std::{net::SocketAddr, process::Command};

pub struct UdpBlackhole(Vec<Rule>);

struct Rule {
    from: SocketAddr,
    to: SocketAddr,
    padded_only: bool,
}

impl UdpBlackhole {
    pub fn new([left, right]: [SocketAddr; 2]) -> Self {
        let mut filter = Self(Vec::new());
        filter.add(left, right, false);
        filter.add(right, left, false);
        filter
    }

    /// Keep the short outbound QNT challenge and the inbound matching response.
    /// Block the padded QUIC 1200-byte validation packet that follows the probe.
    #[allow(dead_code)]
    pub fn block_padded_validation(from: SocketAddr, to: SocketAddr) -> Self {
        let mut filter = Self(Vec::new());
        filter.add(from, to, true);
        filter
    }

    fn add(&mut self, from: SocketAddr, to: SocketAddr, padded_only: bool) {
        let rule = Rule {
            from,
            to,
            padded_only,
        };
        assert!(rule.run("-I"), "install UDP fault inside the lab namespace");
        self.0.push(rule);
    }
}

impl Rule {
    fn run(&self, action: &str) -> bool {
        let (tool, padded_length) = if self.from.is_ipv4() {
            ("iptables", "1228:65535")
        } else {
            ("ip6tables", "1248:65535")
        };
        let mut command = Command::new(tool);
        command.args([
            "-w",
            action,
            "OUTPUT",
            "-o",
            "lo",
            "-p",
            "udp",
            "--sport",
            &self.from.port().to_string(),
            "--dport",
            &self.to.port().to_string(),
        ]);
        if self.padded_only {
            command.args(["-m", "length", "--length", padded_length]);
        }
        command.args(["-j", "DROP"]).status().unwrap().success()
    }
}

impl Drop for UdpBlackhole {
    fn drop(&mut self) {
        for rule in self.0.drain(..) {
            assert!(rule.run("-D"), "remove UDP fault from the lab namespace");
        }
    }
}
