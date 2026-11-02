# First complete SDK story: environment route assessment

Read-only assessment of MIOPUNCH-73's Pixel 6a Browser → Y700 Native run,
2026-10-05 04:42:52–04:43:34 UTC. Its original local receipt is
`/tmp/mio73-public-pixel-y700-bounded-retry-20261005/receipt.json`, SHA256
`603dc22b6ee53851d62c19afe5abef0608cec78f576023a2672943b982bd1ede`.
[The structured assessment](sdk-public-rtc-route-assessment.json) extracts actual
facts and source/artifact identities without raw SDP, secrets or node identities.
No device command, independent STUN request, public TLS probe, network change or
new SDK session was performed for this assessment.

| Layer | Actual evidence | Qualification boundary |
| --- | --- | --- |
| Normal public TLS | EMQX MQTTs `8883` and browser WSS `8084` completed with normal certificate/hostname verification. Native public-root mode, browser WSS and actual MQTT protocol readiness succeeded. | This selected facility and run; no statement about other public services or UDP egress. |
| Complete restricted gate | Admission reached Complete with authorization and secret saved. Both independent identities reopened unchanged. Actual member ALPN `miopunch/session/member/1`, new protocol, expected peer match, current member authorization and ordinary handle were observed; common connect caller took 10910.92ms. | This member direction and frozen artifacts; nonempty replica exchange and other initiator/platform combinations are outside this receipt. |
| Actual RTC data on original session | Original stable session and binding remained unchanged; original QUIC fresh ACK and final-release fresh ACK were observed. RTC was selected with nominated, succeeded, writable ICE and actual traffic counters. SDK PING/PONG completed, with selected-path sent/received counters increasing. | Actual SDK/QUIC/RTC business success; candidate publication filtering does not establish OS routing. |
| Physical NAT without overlay | The actual selected pair matches the earlier physical Wi-Fi addresses and public mappings; native RTC is bound to the physical Y700 address. Prior Wi-Fi/routing and normal system TLS `warp=off` observations corroborate this. | **Supported, unqualified.** No contemporaneous UID/socket-mark/exact-peer OS egress witness was captured for the ended SDK run. Its `physicalWanQualified=false` remains unchanged. |

The browser selected UDP `111.197.237.227:37644`, related to Pixel physical
`192.168.1.104:53189`, against `183.197.94.96:26729`, related to Y700 physical
`192.168.0.107:54041`. Native independently selected local host
`192.168.0.107:54041` against Pixel `111.197.237.227:37644`. These are consistent
views of the NAT pair. Both used only SDK-configured Cloudflare
`162.159.207.0:3478`; no TURN was configured. The workstation supplied only code,
ADB control and evidence.

Earlier Pixel route rules/tables showed Wi-Fi through `192.168.1.1`, with no
tunnel route. Y700 had Wi-Fi through `192.168.0.1` and retained Tailscale for
control; its observed VPN tables carried Tailscale and workstation LAN routes,
without the selected public peer destinations. Earlier UID2000 observer lookups
used Wi-Fi. These observations precede the SDK run. Public address ranges,
server-reflexive candidate types, earlier HTTPS WARP flags and shell-UID routes
do not independently prove the browser's actual UDP route, and current route
reads cannot retroactively become contemporaneous proof for the completed run.

The same run observed real browser/native cleanup: member and network close/free,
zero SDK associations and pumps immediately and after the 100ms sample, completed
owned IndexedDB deletion, owned-tab absence, removed ADB rules, closed listener,
native exit 0 without force, owned PID absence immediately and later, ended
readers and removed owned directory. Native socket retirement remained at its
baseline of six sockets before/immediate/late; this is not a claim of zero OS
sockets or closure of unrelated device resources.

## Independent read-only collector

[android_pair_route_receipt.py](../../../scripts/android_pair_route_receipt.py)
is prepared for a root-coordinated window during an already-authorized live SDK
story. It has not run on either device. It reads only physical IPv4 addresses,
IPv4 rules/all route tables, the exact Chrome package UID or the exact owned
native PID's `Uid:` line, and the selected peer route using the physical source
and observed UID. Package queries retain only the exact test package's result;
the collector does not inspect pages, browsing data or credentials.

The root constructs `--request` JSON from that live SDK's actual diagnostics:

```json
{
  "endpoints": [
    {
      "model": "Pixel_6a",
      "actor": { "kind": "chrome", "package": "com.android.chrome" },
      "selectedTuple": {
        "localAddress": "<actual physical IPv4>",
        "localPort": 1,
        "peerAddress": "<actual selected peer IPv4>",
        "peerPort": 1
      }
    },
    {
      "model": "TB322FC",
      "actor": {
        "kind": "native",
        "ownedDirectory": "/data/local/tmp/miopunch-pair-<exact live actor UUID>"
      },
      "selectedTuple": {
        "localAddress": "<actual physical IPv4>",
        "localPort": 1,
        "peerAddress": "<actual selected peer IPv4>",
        "peerPort": 1
      }
    }
  ]
}
```

Replace every placeholder and example port with actual live facts. Both exact
models may instead be native actors when supplied their own runtime directories.
Socket UID remains unknown: the package/process UID is explicitly identified as
the route-query UID. An omitted socket mark is unknown, never invented zero.
An optional native `socketEvidence` requires the owning fixture's actual
`getsockopt(SO_MARK)` success, numeric value, live PID and matching local tuple:

```json
{
  "operation": "getsockopt(SO_MARK)",
  "success": true,
  "value": 0,
  "pid": 1,
  "localAddress": "<same actual selected physical IPv4>",
  "localPort": 1
}
```

The zero is permitted only when that actual operation observed zero; missing or
failed observation omits `socketEvidence`. The collector never obtains mark or
UID by granting privileges, and never changes a socket or device network.

```sh
rtk proxy python3 -B scripts/android_pair_route_receipt.py \
  --request /tmp/actual-live-sdk-route-request.json \
  --output /tmp/new-live-sdk-route-receipt.json
```

Unsupported UID queries retain original exits and do not fall back to the shell
UID. Unknown UID skips lookup. The collector records evidence and limits with
`physicalWanQualified=false`; independent qualification must correlate it with
the same live SDK tuple and routing policy. If sufficient UID/mark facts cannot
be observed, the uncertainty remains explicit. No extra SDK repetition or public
service test is performed by this collector.

[Four local fake-driver checks](route-collector-local-validation.json) cover
exact package UID, no invented socket UID/mark, owned PID/tuple matching for mark
zero, missing-UID lookup exclusion, original failures and partial evidence,
unowned path rejection, and overwrite rejection before device access. The
already-reviewed runtime/build/page source hashes remain unchanged.
