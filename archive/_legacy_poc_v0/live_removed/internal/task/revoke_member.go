package task

import (
	"errors"
	"os"
	"strings"
	"time"

	"github.com/miopunch/miopunch/internal/controlplane"
	"github.com/miopunch/miopunch/internal/poc"
	"github.com/miopunch/miopunch/internal/pocstate"
)

func (m *Manager) runRevokeMemberTask(taskID string, rawArgs []byte) {
	var args RevokeMemberArgs
	if err := decodeArgs(rawArgs, &args); err != nil {
		m.addFact(taskID, poc.Fact{Message: err.Error()})
		m.addSuggestion(taskID, poc.Suggestion{Message: "retry with valid args"})
		m.done(taskID, poc.ReasonCodeBadRequest, poc.ExitCodeBadRequest)
		return
	}
	args = args.normalize()
	if args.PeerID == "" {
		m.addFact(taskID, poc.Fact{Message: "missing peer_id"})
		m.addSuggestion(taskID, poc.Suggestion{Message: "use: miopunch revoke <peer_id> --dangerous"})
		m.done(taskID, poc.ReasonCodeBadRequest, poc.ExitCodeBadRequest)
		return
	}
	if !args.Dangerous {
		m.addFact(taskID, poc.Fact{Message: "missing --dangerous (revoke is irreversible in POC v0)"})
		m.addSuggestion(taskID, poc.Suggestion{Message: "re-run with: miopunch revoke <peer_id> --dangerous"})
		m.done(taskID, poc.ReasonCodeBadRequest, poc.ExitCodeBadRequest)
		return
	}

	targetPeerID, err := controlplane.CanonicalizePeerID(args.PeerID)
	if err != nil {
		m.addFact(taskID, poc.Fact{Message: "invalid peer_id: " + err.Error()})
		m.addSuggestion(taskID, poc.Suggestion{Message: "retry with a valid peer_id"})
		m.done(taskID, poc.ReasonCodeBadRequest, poc.ExitCodeBadRequest)
		return
	}

	stateDir, err := pocstate.StateDir(m.statePath)
	if err != nil {
		m.addFact(taskID, poc.Fact{Message: "state_dir: " + err.Error()})
		m.addSuggestion(taskID, poc.Suggestion{Message: "retry"})
		m.done(taskID, poc.ReasonCodeInternal, poc.ExitCodeInternal)
		return
	}

	selfID, err := pocstate.EnsureIdentity(stateDir)
	if err != nil {
		m.addFact(taskID, poc.Fact{Message: "ensure identity: " + err.Error()})
		m.addSuggestion(taskID, poc.Suggestion{Message: "retry"})
		m.done(taskID, poc.ReasonCodeInternal, poc.ExitCodeInternal)
		return
	}

	head, err := pocstate.LoadGovernanceHeadSnapshot(stateDir)
	if err != nil {
		if errors.Is(err, os.ErrNotExist) {
			m.addFact(taskID, poc.Fact{Message: "missing governance head snapshot"})
			m.addSuggestion(taskID, poc.Suggestion{Message: "run: miopunch invite (to initialize governance state) and retry"})
			m.done(taskID, poc.ReasonCodeBadRequest, poc.ExitCodeBadRequest)
			return
		}
		m.addFact(taskID, poc.Fact{Message: "load head snapshot: " + err.Error()})
		m.addSuggestion(taskID, poc.Suggestion{Message: "retry"})
		m.done(taskID, poc.ReasonCodeInternal, poc.ExitCodeInternal)
		return
	}

	if !head.IsAdmin(selfID.PeerID) {
		m.addFact(taskID, poc.Fact{Message: "only admins may revoke members"})
		m.addFact(taskID, poc.Fact{Message: "self_peer_id=" + selfID.PeerID})
		m.done(taskID, poc.ReasonCodeForbidden, poc.ExitCodeForbidden)
		return
	}
	if head.IsOwner(targetPeerID) || head.IsAdmin(targetPeerID) {
		m.addFact(taskID, poc.Fact{Message: "cannot revoke owners/admins"})
		m.addFact(taskID, poc.Fact{Message: "target_peer_id=" + targetPeerID})
		m.done(taskID, poc.ReasonCodeForbidden, poc.ExitCodeForbidden)
		return
	}

	if _, err := pocstate.EnsureDecls(stateDir); err != nil {
		m.addFact(taskID, poc.Fact{Message: "ensure decls: " + err.Error()})
		m.addSuggestion(taskID, poc.Suggestion{Message: "retry"})
		m.done(taskID, poc.ReasonCodeInternal, poc.ExitCodeInternal)
		return
	}

	decl, err := pocstate.NewRevokeMemberDeclV0(time.Now().UTC(), selfID, pocstate.RevokeMemberBodyV0{
		MemberPeerID: targetPeerID,
	})
	if err != nil {
		m.addFact(taskID, poc.Fact{Message: "new revoke_member decl: " + err.Error()})
		m.addSuggestion(taskID, poc.Suggestion{Message: "retry"})
		m.done(taskID, poc.ReasonCodeInternal, poc.ExitCodeInternal)
		return
	}

	declsFile, err := pocstate.UpdateDecls(stateDir, func(f *pocstate.DeclsFileV0) error {
		f.Decls = pocstate.AddDeclSetUnionV0(f.Decls, decl)
		return nil
	})
	if err != nil {
		m.addFact(taskID, poc.Fact{Message: "save decls: " + err.Error()})
		m.addSuggestion(taskID, poc.Suggestion{Message: "retry"})
		m.done(taskID, poc.ReasonCodeInternal, poc.ExitCodeInternal)
		return
	}

	headB64 := strings.TrimSpace(declsFile.DeclsHeadB64)

	m.addFact(taskID, poc.Fact{TermID: "revoked_peer_id", Message: "revoked_peer_id=" + targetPeerID})
	m.addFact(taskID, poc.Fact{TermID: "decl_msg_id", Message: "decl_msg_id=" + decl.MsgID})
	if headB64 != "" {
		m.addFact(taskID, poc.Fact{TermID: "decls_head_b64", Message: "decls_head_b64=" + headB64})
	}
	m.addSuggestion(taskID, poc.Suggestion{Message: "revocation is permanent in POC v0; re-join requires a new identity"})
	m.done(taskID, poc.ReasonCodeOK, poc.ExitCodeOK)
}
