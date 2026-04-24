package connectivity

import (
	"context"
	"testing"
)

func TestInternalSTUNBucketsCNContainsVerifiedPublicEndpoints(t *testing.T) {
	cn, global := internalSTUNBuckets()

	if len(global) == 0 {
		t.Fatal("global bucket should not be empty")
	}

	wantCN := []string{
		"udp://106.13.249.54:3478",
		"udp://106.13.248.6:3478",
		"udp://106.12.251.193:3478",
		"udp://111.206.174.3:3478",
		"udp://stun.chat.bilibili.com:3478",
		"udp://stun.douyucdn.cn:18000",
		"udp://stun.hitv.com:3478",
		"udp://106.12.251.31:3478",
		"udp://106.12.251.52:3478",
		"udp://106.12.71.140:3478",
		"udp://180.76.162.88:3478",
		"udp://77.72.169.210:3478",
		"udp://77.72.169.211:3478",
		"udp://77.72.169.212:3478",
		"udp://77.72.169.213:3478",
	}
	wantGlobal := []string{
		"turn.cloudflare.com:3478",
		"udp://stun.cloudflare.com:3478",
		"fwa.lifesizecloud.com:3478",
		"stun.freeswitch.org:3478",
		"stun.voip.blackberry.com:3478",
		"stun.nextcloud.com:3478",
		"udp://stun.sipnet.com:3478",
		"stun.radiojar.com:3478",
		"stun.sonetel.com:3478",
		"stun.sonetel.net:3478",
		"stun.siplogin.de:3478",
		"stun.dcalling.de:3478",
		"stun.flashdance.cx:3478",
		"stun.sip.us:3478",
		"stun.ipfire.org:3478",
		"stun.cope.es:3478",
		"stun.annatel.net:3478",
		"tcp://stun.voipgate.com:3478",
		"stun.hot-chilli.net:3478",
		"u1.xirsys.com:3478",
		"relay1.expressturn.com:3478",
		"relay2.expressturn.com:3478",
		"relay4.expressturn.com:3478",
		"relay5.expressturn.com:3478",
		"relay6.expressturn.com:3478",
		"relay7.expressturn.com:3478",
		"relay8.expressturn.com:3478",
		"stun.relay.metered.ca:80",
		"stun.hivestreaming.com:3478",
		"stun.kinesisvideo.us-east-2.amazonaws.com:443",
		"stun.kinesisvideo.us-east-1.amazonaws.com:443",
		"stun.kinesisvideo.af-south-1.amazonaws.com:443",
		"stun.kinesisvideo.ap-east-1.amazonaws.com:443",
		"stun.kinesisvideo.ap-south-1.amazonaws.com:443",
		"stun.kinesisvideo.ap-northeast-2.amazonaws.com:443",
		"stun.kinesisvideo.ap-southeast-1.amazonaws.com:443",
		"stun.kinesisvideo.ap-southeast-2.amazonaws.com:443",
		"stun.kinesisvideo.ap-northeast-1.amazonaws.com:443",
		"stun.kinesisvideo.ca-central-1.amazonaws.com:443",
		"stun.kinesisvideo.eu-central-1.amazonaws.com:443",
		"stun.kinesisvideo.eu-west-1.amazonaws.com:443",
		"stun.kinesisvideo.eu-west-2.amazonaws.com:443",
		"stun.kinesisvideo.eu-west-3.amazonaws.com:443",
		"stun.kinesisvideo.sa-east-1.amazonaws.com:443",
	}
	wantCNPrefix := []string{
		"udp://106.13.249.54:3478",
		"udp://106.13.248.6:3478",
		"udp://106.12.251.193:3478",
		"udp://111.206.174.3:3478",
		"udp://stun.chat.bilibili.com:3478",
		"udp://stun.douyucdn.cn:18000",
		"udp://stun.hitv.com:3478",
	}
	wantGlobalPrefix := []string{
		"global.turn.twilio.com:3478",
		"turn.cloudflare.com:3478",
		"udp://stun.cloudflare.com:3478",
		"stun.relay.metered.ca:80",
		"fwa.lifesizecloud.com:3478",
		"stun.hivestreaming.com:3478",
	}

	gotCN := make(map[string]struct{}, len(cn))
	for _, server := range cn {
		gotCN[server] = struct{}{}
	}
	for _, server := range wantCN {
		if _, ok := gotCN[server]; !ok {
			t.Fatalf("cn bucket missing %q", server)
		}
	}
	gotGlobal := make(map[string]struct{}, len(global))
	for _, server := range global {
		gotGlobal[server] = struct{}{}
	}
	for _, server := range []string{
		"stun.l.google.com:19302",
		"stun1.l.google.com:19302",
		"stun2.l.google.com:19302",
		"stun3.l.google.com:19302",
		"stun4.l.google.com:19302",
	} {
		if _, ok := gotGlobal[server]; ok {
			t.Fatalf("global bucket contains removed endpoint %q", server)
		}
	}
	for _, server := range wantGlobal {
		if _, ok := gotGlobal[server]; !ok {
			t.Fatalf("global bucket missing %q", server)
		}
	}
	for index, server := range wantCNPrefix {
		if cn[index] != server {
			t.Fatalf("cn[%d] = %q, want %q", index, cn[index], server)
		}
	}
	for index, server := range wantGlobalPrefix {
		if global[index] != server {
			t.Fatalf("global[%d] = %q, want %q", index, global[index], server)
		}
	}

	cn[0] = "mutated"
	global[0] = "mutated-global"

	cn2, _ := internalSTUNBuckets()
	if cn2[0] == "mutated" {
		t.Fatal("internalSTUNBuckets should return a cloned cn bucket")
	}
	_, global2 := internalSTUNBuckets()
	if global2[0] == "mutated-global" {
		t.Fatal("internalSTUNBuckets should return a cloned global bucket")
	}
}

func TestResolveInternalSTUNServersStopsAtEndpointLimit(t *testing.T) {
	servers := []string{
		"1.1.1.1:3478",
		"1.0.0.1:3478",
		"8.8.8.8:3478",
		"8.8.4.4:3478",
		"9.9.9.9:3478",
		"149.112.112.112:3478",
		"stun.example.com:3478",
	}

	got, errs := resolveInternalSTUNServers(context.Background(), nil, servers)
	if len(errs) != 0 {
		t.Fatalf("resolveInternalSTUNServers returned errors: %v", errs)
	}
	if len(got) != internalSTUNResolvedEndpointLimit {
		t.Fatalf("resolved %d endpoints, want %d", len(got), internalSTUNResolvedEndpointLimit)
	}
	for index := 0; index < internalSTUNResolvedEndpointLimit; index++ {
		if got[index] != servers[index] {
			t.Fatalf("resolved[%d] = %q, want %q", index, got[index], servers[index])
		}
	}
}
