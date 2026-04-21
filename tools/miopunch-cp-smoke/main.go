package main

import (
	"context"
	"crypto/ed25519"
	"crypto/sha256"
	"encoding/base32"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"net"
	"os"
	"os/signal"
	"strings"
	"sync"
	"syscall"
	"time"

	mqttclient "github.com/256dpi/gomqtt/client"
	"github.com/256dpi/gomqtt/client/future"
	"github.com/256dpi/gomqtt/packet"

	"github.com/miopunch/miopunch/internal/controlplane"
)

const (
	defaultMQTTURL = "tcp://127.0.0.1:1883"
)

type multiFlag []string

func (m *multiFlag) String() string {
	return strings.Join(*m, ",")
}

func (m *multiFlag) Set(value string) error {
	*m = append(*m, value)
	return nil
}

type smokeConfig struct {
	dstPeerID     string
	meshTimeout   time.Duration
	totalTimeout  time.Duration
	requestBody   string
	respCh        chan controlplane.Message
	wantReplyToID string
}

type node struct {
	ctx context.Context

	netSecret  []byte
	selfPeerID string
	priv       ed25519.PrivateKey

	peers map[string]ed25519.PublicKey // peer_id -> pubkey

	udpConn        *net.UDPConn
	srcNeighborBy  map[string]string // udpAddr.String() -> neighbor peer_id
	meshSendByPeer map[string]controlplane.SendFunc

	mqtt *mqttclient.Client

	fwd *controlplane.Forwarder

	responseDelay time.Duration

	mu    sync.Mutex
	smoke *smokeConfig

	echoReqCount int64
}

func peerIDFromEd25519Pub(pub ed25519.PublicKey) (string, error) {
	if len(pub) != ed25519.PublicKeySize {
		return "", fmt.Errorf("invalid ed25519 public key length: %d", len(pub))
	}
	sum := sha256.Sum256(pub)
	raw := sum[:16]
	id := base32.StdEncoding.WithPadding(base32.NoPadding).EncodeToString(raw)
	return controlplane.CanonicalizePeerID(id)
}

func pubKeyB64URLNoPad(pub ed25519.PublicKey) (string, error) {
	if len(pub) != ed25519.PublicKeySize {
		return "", fmt.Errorf("invalid ed25519 public key length: %d", len(pub))
	}
	return base64.RawURLEncoding.EncodeToString(pub), nil
}

func parseHexBytes(value string, wantLen int) ([]byte, error) {
	trimmed := strings.TrimSpace(value)
	trimmed = strings.TrimPrefix(trimmed, "0x")
	if trimmed == "" {
		return nil, errors.New("empty hex")
	}
	b, err := hex.DecodeString(trimmed)
	if err != nil {
		return nil, err
	}
	if wantLen > 0 && len(b) != wantLen {
		return nil, fmt.Errorf("invalid hex length: got=%d want=%d", len(b), wantLen)
	}
	return b, nil
}

func connectMQTT(ctx context.Context, brokerURL string, clientID string, onMessage func(topic string, payload []byte)) (*mqttclient.Client, error) {
	c := mqttclient.New()
	c.Callback = func(msg *packet.Message, err error) error {
		if err != nil {
			return nil
		}
		if msg == nil {
			return nil
		}
		if onMessage != nil {
			// Copy: the mqtt client owns the backing buffer.
			payload := make([]byte, len(msg.Payload))
			copy(payload, msg.Payload)
			onMessage(msg.Topic, payload)
		}
		return nil
	}

	cfg := mqttclient.NewConfigWithClientID(brokerURL, clientID)
	cfg.CleanSession = true
	cfg.ValidateSubs = true

	f, err := c.Connect(cfg)
	if err != nil {
		return nil, err
	}
	if err := waitMQTTFutureCtx(ctx, f, 5*time.Second); err != nil {
		_ = c.Close()
		return nil, err
	}

	return c, nil
}

func waitMQTTFutureCtx(ctx context.Context, f mqttclient.GenericFuture, fallback time.Duration) error {
	timeout := fallback
	if deadline, ok := ctx.Deadline(); ok {
		remaining := time.Until(deadline)
		if remaining > 0 && remaining < timeout {
			timeout = remaining
		}
	}

	if err := f.Wait(timeout); err != nil {
		if errors.Is(err, future.ErrTimeout) {
			return context.DeadlineExceeded
		}
		return err
	}
	return nil
}

func (n *node) start() error {
	fwd, err := controlplane.NewForwarder(controlplane.ForwarderConfig{
		NetSecret:       n.netSecret,
		SelfPeerID:      n.selfPeerID,
		Seen:            controlplane.NewSeenCache(0, 0, nil),
		Neighbors:       n.meshSendByPeer,
		Deliver:         n.deliver,
		ForwardQueueMax: 0,
	})
	if err != nil {
		return err
	}
	n.fwd = fwd

	go func() {
		<-n.ctx.Done()
		_ = n.udpConn.Close()
	}()

	go n.udpReadLoop()

	return nil
}

func (n *node) close() {
	if n.mqtt != nil {
		_ = n.mqtt.Disconnect(200 * time.Millisecond)
		_ = n.mqtt.Close()
	}
	if n.fwd != nil {
		_ = n.fwd.Close()
	}
	if n.udpConn != nil {
		_ = n.udpConn.Close()
	}
}

func (n *node) udpReadLoop() {
	buf := make([]byte, 64*1024)
	for {
		nRead, addr, err := n.udpConn.ReadFromUDP(buf)
		if err != nil {
			if errors.Is(err, net.ErrClosed) {
				return
			}
			select {
			case <-n.ctx.Done():
				return
			default:
				fmt.Fprintf(os.Stderr, "udp read error: %v\n", err)
				continue
			}
		}

		neighborID, ok := n.srcNeighborBy[addr.String()]
		if !ok {
			fmt.Fprintf(os.Stderr, "udp drop from unknown addr=%s\n", addr.String())
			continue
		}

		payload := make([]byte, nRead)
		copy(payload, buf[:nRead])
		n.fwd.HandleInbound(neighborID, payload)
	}
}

func (n *node) verifyForSelf(msg controlplane.Message) error {
	sender := msg.Signed.SenderPeerID
	pub, ok := n.peers[sender]
	if !ok {
		return fmt.Errorf("unknown sender peer_id: %q", sender)
	}
	return controlplane.VerifyV0ForSelf(n.selfPeerID, pub, msg)
}

func (n *node) deliver(srcNeighborID string, msg controlplane.Message) error {
	if err := n.verifyForSelf(msg); err != nil {
		return err
	}

	switch msg.Signed.Kind {
	case "smoke_echo_req":
		return n.handleEchoReq(srcNeighborID, msg)
	case "smoke_echo_resp":
		return n.handleEchoResp(srcNeighborID, msg)
	default:
		return nil
	}
}

func (n *node) handleEchoReq(srcNeighborID string, msg controlplane.Message) error {
	n.mu.Lock()
	n.echoReqCount++
	count := n.echoReqCount
	delay := n.responseDelay
	n.mu.Unlock()

	fmt.Fprintf(os.Stderr, "echo_req recv: via=%s from=%s msg_id=%s count=%d\n", srcNeighborID, msg.Signed.SenderPeerID, msg.Route.MsgID, count)

	if delay > 0 {
		select {
		case <-n.ctx.Done():
			return n.ctx.Err()
		case <-time.After(delay):
		}
	}

	body, err := json.Marshal(struct {
		OK    bool  `json:"ok"`
		Count int64 `json:"count"`
	}{OK: true, Count: count})
	if err != nil {
		return err
	}

	ct, _, err := n.newCiphertext(msg.Signed.SenderPeerID, "smoke_echo_resp", msg.Route.MsgID, body)
	if err != nil {
		return err
	}

	// If the request arrived via MQTT fallback, reply via MQTT so the caller
	// can complete the request/response flow without requiring a mesh path.
	if srcNeighborID == "mqtt" {
		if err := n.publishMQTT(msg.Signed.SenderPeerID, ct); err != nil {
			return fmt.Errorf("mqtt reply publish: %w", err)
		}
		return nil
	}

	n.sendMesh(ct)
	return nil
}

func (n *node) handleEchoResp(srcNeighborID string, msg controlplane.Message) error {
	fmt.Fprintf(os.Stderr, "echo_resp recv: via=%s from=%s in_reply_to=%s msg_id=%s\n", srcNeighborID, msg.Signed.SenderPeerID, msg.Signed.InReplyTo, msg.Route.MsgID)

	n.mu.Lock()
	smoke := n.smoke
	n.mu.Unlock()
	if smoke == nil {
		return nil
	}
	if msg.Signed.InReplyTo != smoke.wantReplyToID {
		return nil
	}

	select {
	case smoke.respCh <- msg:
	default:
	}
	return nil
}

func (n *node) newCiphertext(dstPeerID string, kind string, inReplyTo string, bodyJSON []byte) ([]byte, string, error) {
	msgID, err := controlplane.NewMsgID()
	if err != nil {
		return nil, "", err
	}
	nowMS := time.Now().UTC().UnixMilli()

	m := controlplane.Message{
		ProtoVersion: controlplane.ProtoVersionV0,
		Route: controlplane.Route{
			DstPeerID:       dstPeerID,
			MsgID:           msgID,
			HopLimit:        controlplane.HopLimitMax,
			CreatedAtUnixMs: nowMS,
		},
		Signed: controlplane.Signed{
			SenderPeerID: n.selfPeerID,
			Kind:         kind,
			InReplyTo:    inReplyTo,
			Body:         bodyJSON,
		},
	}
	if err := controlplane.SignV0(n.priv, &m); err != nil {
		return nil, "", err
	}

	pt, err := controlplane.MarshalMessage(m)
	if err != nil {
		return nil, "", err
	}
	ct, err := controlplane.SealGroupV0(n.netSecret, pt)
	if err != nil {
		return nil, "", err
	}
	return ct, msgID, nil
}

func (n *node) sendMesh(ciphertext []byte) {
	for neighborID, send := range n.meshSendByPeer {
		if send == nil {
			continue
		}
		if err := send(ciphertext); err != nil {
			fmt.Fprintf(os.Stderr, "mesh send error: neighbor=%s err=%v\n", neighborID, err)
		}
	}
}

func (n *node) publishMQTT(dstPeerID string, ciphertext []byte) error {
	if n.mqtt == nil {
		return errors.New("mqtt is disabled")
	}
	topic, err := controlplane.DeriveInboxTopic(n.netSecret, dstPeerID)
	if err != nil {
		return err
	}
	f, err := n.mqtt.Publish(topic, ciphertext, packet.QOSAtLeastOnce, false)
	if err != nil {
		return err
	}
	return waitMQTTFutureCtx(n.ctx, f, 3*time.Second)
}

func (n *node) runSmoke(dstPeerID string, meshTimeout time.Duration, totalTimeout time.Duration, requestBody string) error {
	body, err := json.Marshal(struct {
		Body string `json:"body"`
	}{Body: requestBody})
	if err != nil {
		return err
	}

	ct, reqID, err := n.newCiphertext(dstPeerID, "smoke_echo_req", "", body)
	if err != nil {
		return err
	}

	respCh := make(chan controlplane.Message, 1)

	n.mu.Lock()
	n.smoke = &smokeConfig{
		dstPeerID:     dstPeerID,
		meshTimeout:   meshTimeout,
		totalTimeout:  totalTimeout,
		requestBody:   requestBody,
		respCh:        respCh,
		wantReplyToID: reqID,
	}
	n.mu.Unlock()

	defer func() {
		n.mu.Lock()
		n.smoke = nil
		n.mu.Unlock()
	}()

	fmt.Fprintf(os.Stderr, "smoke: sending mesh req msg_id=%s dst=%s\n", reqID, dstPeerID)
	n.sendMesh(ct)

	select {
	case <-respCh:
		fmt.Fprintf(os.Stderr, "smoke: response received (mesh-first)\n")
		return nil
	case <-time.After(meshTimeout):
	}

	fmt.Fprintf(os.Stderr, "smoke: mesh timeout after %s; publishing MQTT fallback\n", meshTimeout)
	if err := n.publishMQTT(dstPeerID, ct); err != nil {
		return fmt.Errorf("mqtt fallback publish: %w", err)
	}

	deadline := time.NewTimer(totalTimeout - meshTimeout)
	defer deadline.Stop()
	select {
	case <-respCh:
		fmt.Fprintf(os.Stderr, "smoke: response received (after MQTT fallback)\n")
		return nil
	case <-deadline.C:
		return fmt.Errorf("smoke: timeout after %s waiting for response", totalTimeout)
	case <-n.ctx.Done():
		return n.ctx.Err()
	}
}

func parseNeighbor(value string) (peerID string, addr string, _ error) {
	before, after, ok := strings.Cut(value, "=")
	if !ok {
		return "", "", fmt.Errorf("invalid neighbor %q (want PEER_ID=IP:PORT)", value)
	}
	peerID = strings.TrimSpace(before)
	addr = strings.TrimSpace(after)
	if peerID == "" || addr == "" {
		return "", "", fmt.Errorf("invalid neighbor %q (empty peer_id or addr)", value)
	}
	return peerID, addr, nil
}

func parsePeer(value string) (peerID string, pubB64 string, _ error) {
	before, after, ok := strings.Cut(value, ":")
	if !ok {
		return "", "", fmt.Errorf("invalid peer %q (want PEER_ID:PUBKEY_B64URL)", value)
	}
	peerID = strings.TrimSpace(before)
	pubB64 = strings.TrimSpace(after)
	if peerID == "" || pubB64 == "" {
		return "", "", fmt.Errorf("invalid peer %q (empty peer_id or pubkey)", value)
	}
	return peerID, pubB64, nil
}

func main() {
	var neighbors multiFlag
	var peers multiFlag
	var peerSeeds multiFlag

	netSecretHex := flag.String("net-secret-hex", "", "32B network secret (hex)")
	seedHex := flag.String("seed-hex", "", "32B ed25519 seed (hex) for this node")
	selfPeerIDFlag := flag.String("self-peer-id", "", "optional self peer_id (must match derived from seed)")
	listenUDP := flag.String("listen-udp", "", "UDP listen addr (e.g. 0.0.0.0:9001)")
	flag.Var(&neighbors, "neighbor", "mesh neighbor (repeatable): PEER_ID=IP:PORT")
	flag.Var(&peers, "peer", "known peer pubkey (repeatable): PEER_ID:PUBKEY_B64URL")
	flag.Var(&peerSeeds, "peer-seed-hex", "known peer seed (repeatable): 32B hex; derives peer_id+pubkey")

	mqttURL := flag.String("mqtt-url", defaultMQTTURL, "MQTT broker URL (gomqtt format, e.g. tcp://127.0.0.1:1883)")
	mqttClientID := flag.String("mqtt-client-id", "", "optional MQTT client id")
	mqttDisable := flag.Bool("mqtt-disable", false, "disable MQTT subscribe/publish")

	responseDelay := flag.Duration("response-delay", 0, "delay before responding to smoke_echo_req (to trigger fallback)")
	exitAfter := flag.Duration("exit-after", 0, "exit after duration (0 = run until signal)")
	printIdentity := flag.Bool("print-identity", false, "print derived peer_id + pubkey and exit")

	smoke := flag.Bool("smoke", false, "run smoke request (node A)")
	smokeDst := flag.String("smoke-dst-peer-id", "", "smoke destination peer_id (node C)")
	smokeMeshTimeout := flag.Duration("smoke-mesh-timeout", 1*time.Second, "mesh-first timeout before MQTT fallback (spec: 1s)")
	smokeTotalTimeout := flag.Duration("smoke-total-timeout", 6*time.Second, "overall smoke timeout (includes mesh timeout)")
	smokeBody := flag.String("smoke-body", "hello", "request body string for smoke_echo_req")

	flag.Parse()

	netSecret, err := parseHexBytes(*netSecretHex, 32)
	if err != nil {
		fmt.Fprintf(os.Stderr, "invalid --net-secret-hex: %v\n", err)
		os.Exit(2)
	}
	seed, err := parseHexBytes(*seedHex, 32)
	if err != nil {
		fmt.Fprintf(os.Stderr, "invalid --seed-hex: %v\n", err)
		os.Exit(2)
	}

	priv := ed25519.NewKeyFromSeed(seed)
	pub := priv.Public().(ed25519.PublicKey)
	derivedPeerID, err := peerIDFromEd25519Pub(pub)
	if err != nil {
		fmt.Fprintf(os.Stderr, "derive peer_id: %v\n", err)
		os.Exit(2)
	}

	selfPeerID := derivedPeerID
	if strings.TrimSpace(*selfPeerIDFlag) != "" {
		canonical, err := controlplane.CanonicalizePeerID(*selfPeerIDFlag)
		if err != nil {
			fmt.Fprintf(os.Stderr, "invalid --self-peer-id: %v\n", err)
			os.Exit(2)
		}
		if canonical != derivedPeerID {
			fmt.Fprintf(os.Stderr, "--self-peer-id=%s does not match derived=%s\n", canonical, derivedPeerID)
			os.Exit(2)
		}
		selfPeerID = canonical
	}

	selfPubB64, err := pubKeyB64URLNoPad(pub)
	if err != nil {
		fmt.Fprintf(os.Stderr, "encode pubkey: %v\n", err)
		os.Exit(2)
	}

	inboxTopic, err := controlplane.DeriveInboxTopic(netSecret, selfPeerID)
	if err != nil {
		fmt.Fprintf(os.Stderr, "derive inbox topic: %v\n", err)
		os.Exit(2)
	}

	fmt.Fprintf(os.Stderr, "self_peer_id=%s\n", selfPeerID)
	fmt.Fprintf(os.Stderr, "self_pub_b64url=%s\n", selfPubB64)
	fmt.Fprintf(os.Stderr, "inbox_topic=%s\n", inboxTopic)

	if *printIdentity {
		return
	}

	if strings.TrimSpace(*listenUDP) == "" {
		fmt.Fprintf(os.Stderr, "--listen-udp is required\n")
		os.Exit(2)
	}
	udpListenAddr, err := net.ResolveUDPAddr("udp", *listenUDP)
	if err != nil {
		fmt.Fprintf(os.Stderr, "resolve --listen-udp: %v\n", err)
		os.Exit(2)
	}
	udpConn, err := net.ListenUDP("udp", udpListenAddr)
	if err != nil {
		fmt.Fprintf(os.Stderr, "listen udp: %v\n", err)
		os.Exit(2)
	}

	peerPubByID := make(map[string]ed25519.PublicKey)
	peerPubByID[selfPeerID] = pub

	for _, seedHex := range peerSeeds {
		b, err := parseHexBytes(seedHex, 32)
		if err != nil {
			fmt.Fprintf(os.Stderr, "invalid --peer-seed-hex: %v\n", err)
			os.Exit(2)
		}
		p := ed25519.NewKeyFromSeed(b).Public().(ed25519.PublicKey)
		id, err := peerIDFromEd25519Pub(p)
		if err != nil {
			fmt.Fprintf(os.Stderr, "derive peer_id from --peer-seed-hex: %v\n", err)
			os.Exit(2)
		}
		if existing, ok := peerPubByID[id]; ok && !ed25519.PublicKey(existing).Equal(p) {
			fmt.Fprintf(os.Stderr, "peer_id collision with different pubkey: %s\n", id)
			os.Exit(2)
		}
		peerPubByID[id] = p
	}

	for _, raw := range peers {
		idRaw, pubB64, err := parsePeer(raw)
		if err != nil {
			fmt.Fprintf(os.Stderr, "%v\n", err)
			os.Exit(2)
		}
		id, err := controlplane.CanonicalizePeerID(idRaw)
		if err != nil {
			fmt.Fprintf(os.Stderr, "invalid --peer peer_id %q: %v\n", idRaw, err)
			os.Exit(2)
		}
		pubBytes, err := base64.RawURLEncoding.DecodeString(pubB64)
		if err != nil {
			fmt.Fprintf(os.Stderr, "invalid --peer pubkey b64url: %v\n", err)
			os.Exit(2)
		}
		if len(pubBytes) != ed25519.PublicKeySize {
			fmt.Fprintf(os.Stderr, "invalid --peer pubkey length: %d\n", len(pubBytes))
			os.Exit(2)
		}
		pk := ed25519.PublicKey(pubBytes)
		if existing, ok := peerPubByID[id]; ok && !ed25519.PublicKey(existing).Equal(pk) {
			fmt.Fprintf(os.Stderr, "peer_id collision with different pubkey: %s\n", id)
			os.Exit(2)
		}
		peerPubByID[id] = pk
	}

	meshSendByPeerID := make(map[string]controlplane.SendFunc)
	srcNeighborByAddr := make(map[string]string)

	for _, raw := range neighbors {
		neighborIDRaw, addrRaw, err := parseNeighbor(raw)
		if err != nil {
			fmt.Fprintf(os.Stderr, "%v\n", err)
			os.Exit(2)
		}
		neighborID, err := controlplane.CanonicalizePeerID(neighborIDRaw)
		if err != nil {
			fmt.Fprintf(os.Stderr, "invalid neighbor peer_id %q: %v\n", neighborIDRaw, err)
			os.Exit(2)
		}
		udpAddr, err := net.ResolveUDPAddr("udp", addrRaw)
		if err != nil {
			fmt.Fprintf(os.Stderr, "resolve neighbor addr %q: %v\n", addrRaw, err)
			os.Exit(2)
		}

		meshSendByPeerID[neighborID] = func(addr *net.UDPAddr) controlplane.SendFunc {
			return func(ciphertext []byte) error {
				_, err := udpConn.WriteToUDP(ciphertext, addr)
				return err
			}
		}(udpAddr)

		srcNeighborByAddr[udpAddr.String()] = neighborID
	}

	ctx, cancel := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer cancel()

	if *exitAfter > 0 {
		ctx2, cancel2 := context.WithTimeout(ctx, *exitAfter)
		defer cancel2()
		ctx = ctx2
	}

	n := &node{
		ctx:            ctx,
		netSecret:      netSecret,
		selfPeerID:     selfPeerID,
		priv:           priv,
		peers:          peerPubByID,
		udpConn:        udpConn,
		srcNeighborBy:  srcNeighborByAddr,
		meshSendByPeer: meshSendByPeerID,
		responseDelay:  *responseDelay,
		echoReqCount:   0,
		mqtt:           nil,
		fwd:            nil,
	}

	if !*mqttDisable {
		clientID := strings.TrimSpace(*mqttClientID)
		if clientID == "" {
			clientID = fmt.Sprintf("miopunch-cp-smoke-%s-%d", selfPeerID, time.Now().UTC().UnixNano())
		}

		mc, err := connectMQTT(ctx, *mqttURL, clientID, func(topic string, payload []byte) {
			if topic != inboxTopic {
				return
			}
			if n.fwd != nil {
				// Avoid blocking the MQTT client's internal read loop. Deliver handlers may publish,
				// which needs PUBACK processing in the background.
				go n.fwd.HandleInbound("mqtt", payload)
			}
		})
		if err != nil {
			fmt.Fprintf(os.Stderr, "mqtt connect: %v\n", err)
			os.Exit(2)
		}
		n.mqtt = mc

		f, err := n.mqtt.Subscribe(inboxTopic, packet.QOSAtLeastOnce)
		if err != nil {
			fmt.Fprintf(os.Stderr, "mqtt subscribe: %v\n", err)
			os.Exit(2)
		}
		if err := waitMQTTFutureCtx(ctx, f, 5*time.Second); err != nil {
			fmt.Fprintf(os.Stderr, "mqtt subscribe wait: %v\n", err)
			os.Exit(2)
		}
		fmt.Fprintf(os.Stderr, "mqtt subscribed: %s\n", inboxTopic)
	}

	if err := n.start(); err != nil {
		fmt.Fprintf(os.Stderr, "node start: %v\n", err)
		os.Exit(2)
	}
	defer n.close()

	if *smoke {
		dstRaw := strings.TrimSpace(*smokeDst)
		if dstRaw == "" {
			fmt.Fprintf(os.Stderr, "--smoke-dst-peer-id is required with --smoke\n")
			os.Exit(2)
		}
		dstPeerID, err := controlplane.CanonicalizePeerID(dstRaw)
		if err != nil {
			fmt.Fprintf(os.Stderr, "invalid --smoke-dst-peer-id: %v\n", err)
			os.Exit(2)
		}
		if *smokeTotalTimeout <= *smokeMeshTimeout {
			fmt.Fprintf(os.Stderr, "--smoke-total-timeout must be > --smoke-mesh-timeout\n")
			os.Exit(2)
		}

		if err := n.runSmoke(dstPeerID, *smokeMeshTimeout, *smokeTotalTimeout, *smokeBody); err != nil {
			fmt.Fprintf(os.Stderr, "%v\n", err)
			os.Exit(1)
		}

		stats := n.fwd.Stats()
		fmt.Fprintf(os.Stderr, "facts: mesh_forward_drops=%d decrypt_drops=%d decode_drops=%d hop_limit_drops=%d dedup_drops=%d deliver_drops=%d send_errors=%d\n",
			stats.MeshForwardDrops, stats.DecryptDrops, stats.DecodeDrops, stats.HopLimitDrops, stats.DedupDrops, stats.DeliverDrops, stats.SendErrors)
		return
	}

	<-ctx.Done()

	if n.fwd != nil {
		stats := n.fwd.Stats()
		fmt.Fprintf(os.Stderr, "facts: mesh_forward_drops=%d decrypt_drops=%d decode_drops=%d hop_limit_drops=%d dedup_drops=%d deliver_drops=%d send_errors=%d\n",
			stats.MeshForwardDrops, stats.DecryptDrops, stats.DecodeDrops, stats.HopLimitDrops, stats.DedupDrops, stats.DeliverDrops, stats.SendErrors)
	}
}
