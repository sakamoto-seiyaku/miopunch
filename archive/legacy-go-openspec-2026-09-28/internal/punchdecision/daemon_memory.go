package punchdecision

import (
	"sync"
	"time"

	"github.com/miopunch/miopunch/internal/wire"
)

const (
	daemonAnalyzerTTL      = 2 * time.Hour
	daemonCleanInterval    = 10 * time.Minute
	daemonDefaultScopeHint = "sid"
)

var (
	daemonOnce      sync.Once
	daemonMu        sync.Mutex
	daemonEngine    *Engine
	daemonLastClean time.Time
)

func daemonEngineInstance() *Engine {
	daemonOnce.Do(func() {
		daemonEngine = NewEngine(daemonAnalyzerTTL)
		daemonLastClean = time.Now()
	})
	return daemonEngine
}

func daemonMaybeClean() {
	daemonMu.Lock()
	defer daemonMu.Unlock()
	if daemonEngine == nil {
		return
	}
	if time.Since(daemonLastClean) < daemonCleanInterval {
		return
	}
	_, _ = daemonEngine.Clean()
	daemonLastClean = time.Now()
}

// AnalyzeWithDaemonMemory runs the decision engine using daemon-lifetime analyzer
// memory. The caller supplies a stable remote peer identifier used to scope
// analyzer records.
//
// When remotePeerID is empty, the SID is used as a conservative scope to avoid
// sharing analyzer memory across unrelated sessions.
func AnalyzeWithDaemonMemory(sid string, remotePeerID string, visitor *wire.NatHoleVisitor, client *wire.NatHoleClient) (*Result, error) {
	scopeKey := remotePeerID
	if scopeKey == "" {
		scopeKey = daemonDefaultScopeHint + ":" + sid
	}
	daemonMaybeClean()
	return daemonEngineInstance().AnalyzeWithScope(sid, scopeKey, visitor, client)
}

// UDPAnalyzerKey derives the daemon-lifetime UDP analyzer key for a local peer scope.
func UDPAnalyzerKey(remotePeerID string, analysisKey string) string {
	return scopedAnalyzerKey(remotePeerID, "udp", analysisKey)
}

// ReportDaemonUDPSuccess records a successful UDP punching mode/index into the
// daemon-lifetime analyzer memory.
func ReportDaemonUDPSuccess(res *Result) {
	if res == nil || res.AnalyzerKey == "" {
		return
	}
	daemonMaybeClean()
	daemonEngineInstance().ReportSuccess(res.AnalyzerKey, res.Mode, res.Index)
}

// ReportDaemonTCPSuccess records a successful TCP punching mode/index into the
// daemon-lifetime analyzer memory.
func ReportDaemonTCPSuccess(res *Result) {
	if res == nil || res.TCPAnalyzerKey == "" {
		return
	}
	daemonMaybeClean()
	daemonEngineInstance().ReportSuccess(res.TCPAnalyzerKey, res.TCPMode, res.TCPIndex)
}
