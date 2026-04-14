package connectivity

// Internal STUN defaults are used only when the user does not explicitly
// configure STUN via CLI/YAML. They are partitioned into `cn` and `global`
// buckets to support P3.5 cn/global view sampling and arbitration.
//
// Source baseline: gonc.
var (
	internalSTUNCN = []string{
		"stun.miwifi.com:3478",
	}

	internalSTUNGlobal = []string{
		"stun.l.google.com:19302",
		"stun1.l.google.com:19302",
		"stun2.l.google.com:19302",
		"stun3.l.google.com:19302",
		"stun4.l.google.com:19302",
		"global.turn.twilio.com:3478",
		"turn.cloudflare.com:3478",
		"stun.nextcloud.com:443",
	}
)

func internalSTUNBuckets() (cn []string, global []string) {
	cn = append([]string(nil), internalSTUNCN...)
	global = append([]string(nil), internalSTUNGlobal...)
	return cn, global
}

