CASE_ID=p2-04-v6-fallback-direct-ipv4
# IPv6 candidates exist but are not reachable; fallback to IPv4 direct (portmap) should succeed.
A_PROFILE=nat3
B_PROFILE=nat3
ENABLE_IPV6=1
BLOCK_FORWARD_UDP6=1

