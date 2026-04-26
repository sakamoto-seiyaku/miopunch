package coordinator

import (
	"github.com/miopunch/miopunch/internal/punchdecision"
	"github.com/miopunch/miopunch/internal/wire"
)

// AnalyzeOnce delegates to punchdecision.AnalyzeOnce.
//
// New non-lab callers should import internal/punchdecision directly.
func AnalyzeOnce(sid string, visitor *wire.NatHoleVisitor, client *wire.NatHoleClient) (*wire.NatHoleResp, *wire.NatHoleResp, error) {
	return punchdecision.AnalyzeOnce(sid, visitor, client)
}
