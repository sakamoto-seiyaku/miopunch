package connectivity

// Internal STUN defaults are used only when the user does not explicitly
// configure STUN via CLI/YAML. They are partitioned into `cn` and `global`
// buckets to support P3.5 cn/global view sampling and arbitration.
//
// Source baseline: gonc.
var (
	internalSTUNCN = []string{
		// Source: case1 public verification on 2026-04-14
		// (Android mobile network <-> home broadband).
		//
		// Front-load the exact IP literals that were proven to work in case1 so
		// the bounded-concurrency sampler can succeed on Android mobile networks
		// before the per-view timeout budget is exhausted.
		"106.13.249.54:3478",
		"106.13.248.6:3478",
		"106.12.251.193:3478",
		"124.221.129.2:3478",
		"124.222.69.57:3478",
		"111.206.174.3:3478",

		// Keep the bilibili hostname plus supplemental verified IP literals
		// because the built-in resolver currently expands at most two A records
		// per hostname.
		"stun.chat.bilibili.com:3478",
		// Source: natmap issue #18 + MikeWang000000 comment
		// https://github.com/heiher/natmap/issues/18#issuecomment-2093158186
		//
		// Current host-side UDP STUN probe on 2026-04-14:
		// - stun.douyucdn.cn:18000 => 3/3 A records responded
		// - stun.hitv.com:3478 => 3/3 A records responded
		// - stun.cdnbye.com:3478 => 4/6 A records responded
		"stun.douyucdn.cn:18000",
		"stun.hitv.com:3478",
		"stun.cdnbye.com:3478",
		"stun.miwifi.com:3478",

		// Source: natmap issue #18 wray-lee comment
		// https://github.com/heiher/natmap/issues/18#issuecomment-2230898238
		//
		// User requested to keep stun.yy.com in the CN defaults even though the
		// current host-side probe was not stable on 2026-04-14.
		"stun.yy.com:3478",

		// Verified IP supplements for hostname pools that return more usable A
		// records than the current resolver expansion limit can cover.
		"106.12.251.31:3478",
		"106.12.251.52:3478",
		"106.12.71.140:3478",
		"180.76.162.88:3478",
		"111.206.174.2:3478",
		"77.72.169.210:3478",
		"77.72.169.211:3478",
		"77.72.169.212:3478",
		"77.72.169.213:3478",
	}

	internalSTUNGlobal = []string{
		// Source baseline: gonc.
		"global.turn.twilio.com:3478",

		// Source: natmap issue #18 + MikeWang000000 comment
		// https://github.com/heiher/natmap/issues/18#issuecomment-2093158186
		//
		// Current host-side UDP STUN probe on 2026-04-14 confirmed these
		// hostnames respond on the comment-provided UDP-compatible ports.
		"turn.cloudflare.com:3478",
		"stun.cloudflare.com:3478",
		"stun.relay.metered.ca:80",
		"fwa.lifesizecloud.com:3478",
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
		"u1.xirsys.com:3478",
		"relay1.expressturn.com:3478",
		"relay2.expressturn.com:3478",
		"relay4.expressturn.com:3478",
		"relay5.expressturn.com:3478",
		"relay6.expressturn.com:3478",
		"relay7.expressturn.com:3478",
		"relay8.expressturn.com:3478",
		// Source: natmap issue #18 root list
		// https://github.com/heiher/natmap/issues/18
		//
		// Current host-side UDP STUN probe on 2026-04-14 confirmed these
		// hostnames respond on their listed/default UDP STUN ports.
		"stun.nextcloud.com:3478",
		"stun.freeswitch.org:3478",
		"stun.sonetel.com:3478",
		"stun.voip.blackberry.com:3478",
		"stun.voipgate.com:3478",
		"stun.hot-chilli.net:3478",
		"stun.sipnet.com:3478",
		"stun.radiojar.com:3478",
		"stun.l.google.com:19302",
		"stun1.l.google.com:19302",
		"stun2.l.google.com:19302",
		"stun3.l.google.com:19302",
		"stun4.l.google.com:19302",
		"stun.nextcloud.com:443",
	}
)

func internalSTUNBuckets() (cn []string, global []string) {
	cn = append([]string(nil), internalSTUNCN...)
	global = append([]string(nil), internalSTUNGlobal...)
	return cn, global
}
