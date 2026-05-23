## 1. Pre-flight Validation

- [ ] 1.1 Run baseline `export PATH=/usr/local/go/bin:$PATH && go test ./...`

## 2. Presence publish (retained + LWT)

- [ ] 2.1 Add presence topic derivation under `mp/v1/net/<net_root>/presence/<peer_id>`
- [ ] 2.2 Publish retained `online` after MQTT connect
- [ ] 2.3 Configure LWT retained `offline`

## 3. Presence subscribe (Discover)

- [ ] 3.1 Subscribe `.../presence/+` and maintain an in-memory peer list
- [ ] 3.2 GUI: render peer list + online/offline + last ts

## 4. Post-change Validation

- [ ] 4.1 Re-run `go test ./...`
