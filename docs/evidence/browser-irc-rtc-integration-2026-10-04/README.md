# IRC / RTC semantic integration

Fixed released baseline `ca3be8bad6887c96022297081ddeafa2e2514364`.
Final IRC commits `b1fd4d92` and `517b71bb` were integrated together; the latter is the final amended child of the former. Cargo TLS dependencies and browser features were combined, preserving current RTC minima, gate revision/watch fields, exact RTC registration and request deadlines. The main trust helper and S2 caller-close drain fixture were preserved.

[Validation](validation.json) includes all captured source/artifact hashes, native112pass plus one existing ignore, actual fresh builds and ownership cleanup. [IRC](irc-datagram.json) is the real600ms slow datagram profile. [MQTT](wss-restricted-handshake.json) is restricted A. [RTC](relay-member-rtc.json) is the original Ordinary relay M, fresh ACK/stream lifetime/drop/short/pump regression. They do not qualify STEP6 WSS member release. IRC's measured restricted-profile stop branch remains unchanged and unqualified.

Final two-axis review is scheduled with STEP6 against the released baseline and includes conflict resolution; standalone unchanged IRC modules already have independent reviews. Setup errors and exact raw paths remain in validation.
