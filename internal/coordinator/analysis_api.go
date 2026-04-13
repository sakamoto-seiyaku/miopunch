package coordinator

import (
	"time"

	"github.com/miopunch/miopunch/internal/wire"
)

// AnalyzeOnce runs the same NAT-hole analysis logic used by the coordinator
// without requiring a running coord server. This is intended for P3.5
// experiments where a peer (visitor leader) performs analysis after exchanging
// the program-defined inputs via an external signaling channel (e.g. MQTT).
func AnalyzeOnce(sid string, visitor *wire.NatHoleVisitor, client *wire.NatHoleClient) (*wire.NatHoleResp, *wire.NatHoleResp, error) {
	ctrl, err := NewController(24 * time.Hour)
	if err != nil {
		return nil, nil, err
	}
	s := &Session{
		sid:        sid,
		visitorMsg: visitor,
		clientMsg:  client,
	}
	return ctrl.analysis(s)
}
