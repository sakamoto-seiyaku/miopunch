#!/usr/bin/env python3
"""MIO83 thin public SDK admission/replica story, with private control replies.

Import and --development-checks perform no device or public network operation.
Root owns device/public execution after the guarded SDK candidate handoff.
The first admission uses protected public MQTT; subsequent data uses an explicit
allowed path. Every required phase and natural cleanup gates the final result.
"""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[1]
FIXTURE = ROOT / "browser-probe/harness/admission-commit-fixture.mjs"
STRICT_ADMISSION_ALPN = "miopunch/session/admission/1"
STRICT_PRECODE_ALPN = "miopunch/session/precode/1"
STRICT_REPLICA_ALPN = "miopunch/session/replica/1"
ADMISSION_BASELINE_SOURCE_SHA256 = "6e5ebbe3bbaa73872129cb7c5796949dd610b21da79e09c0bbe4cb2eebaf85a6"
ADMISSION_BASELINE_BUILD_SHA256 = "514b91992ea90cb68a0b8f003c9fa264e6808367dba6ca0839acc53a4f69384c"


class EvidenceCheckFailure(RuntimeError):
    """Only runner-owned assertion text is safe for the public failure receipt."""
    def __init__(self, message):
        super().__init__(message)
        self.check_id = "require-" + hashlib.sha256(message.encode()).hexdigest()[:16]
        self.safe_reason = message


def require(condition, message):
    if not condition:
        raise EvidenceCheckFailure(message)


def remember_first_failure(report, error, phase):
    if "firstFailure" in report:
        return
    # Never copy a transport/SDK/ADB exception string containing private values.
    report["firstFailure"] = {
        "checkId": error.check_id if isinstance(error, EvidenceCheckFailure) else phase,
        "phase": phase,
        "safeReason": error.safe_reason if isinstance(error, EvidenceCheckFailure)
            else "operation raised " + type(error).__name__ + "; private details are not published",
        "failureClass": type(error).__name__,
    }


def native_operation_facts(event, generation):
    if event.get("event") not in {"admission-accepted", "admission-ended", "replica-finished"}:
        return None
    known_alpns = {STRICT_ADMISSION_ALPN, STRICT_PRECODE_ALPN, STRICT_REPLICA_ALPN}
    return {"generation":generation,"event":event["event"],
        "alpn":event.get("alpn") if event.get("alpn") in known_alpns else None,
        "callerElapsedMs":event.get("callerElapsedMs") if type(event.get("callerElapsedMs")) is int
            and event["callerElapsedMs"] >= 0 else None,
        "callerBudgetMs":event.get("callerBudgetMs") if type(event.get("callerBudgetMs")) is int
            and event["callerBudgetMs"] >= 0 else None,
        "callerDeadlinePresent":event.get("callerDeadlinePresent")
            if type(event.get("callerDeadlinePresent")) is bool else None}


def keep_provider_cleanup(report, generation, cleanup, checkpoint):
    # These are the real actor/SDK measurements, not a story-stage duration.
    report.setdefault("providerGenerationCleanup",[]).append({"generation":generation,
        "checkpoint":checkpoint,"cleanup":json.loads(json.dumps(cleanup))})


def evidence_exception_class(error):
    name = type(error).__name__
    return name if name in {"EvidenceCheckFailure","RuntimeError","TimeoutError","OSError",
        "FileExistsError","PermissionError","ValueError","TypeError"} else "OtherException"


def current_native_stderr(actor):
    if actor is None:
        return None
    with actor.condition:
        run = actor.runs[-1] if actor.runs else {}
        # These are the existing owned reader's bounds, not another observer.
        return {"generation":len(actor.runs),"ownedGeneration":run.get("generation"),
            "lines":[line[:4096] for line in actor.stderr[:64]],
            "localReadersEnded":run.get("cleanup",{}).get("localReadersEnded")}


def current_native_events(actor):
    if actor is None:
        return None
    with actor.condition:
        run = actor.runs[-1] if actor.runs else {}
        return {"generation":len(actor.runs),"ownedGeneration":run.get("generation"),
            "events":list(actor.events[-512:])}


def native_command(actor, operation, expected, replies, **values):
    matches = expected if callable(expected) else lambda event:event.get("event") == expected
    with actor.condition:
        prior_errors = [event for event in actor.events if event.get("event") == "operator-error"]
    reply = actor.command(json.dumps({"op":operation,**values}),lambda event:matches(event)
        or (event.get("event") == "operator-error" and event.get("operation") == operation
            and not any(event is prior for prior in prior_errors)))
    replies.append({"generation":len(actor.runs),"operation":operation,"reply":reply})
    if len(replies) > 512:
        del replies[0]
    if reply.get("event") == "operator-error" and reply.get("operation") == operation:
        raise RuntimeError(reply.get("failure") if isinstance(reply.get("failure"),str)
            else "owned Native operator returned an error without failure text")
    return reply


def persist_command_evidence(output, devices, actor, checkpoint, *, stage_error=None, phase=None,
        command_replies=()):
    """Retain exact helper-collected commands privately; publish only hashes/counts/exits."""
    device_commands = {role:device.commands for role,device in devices.items() if device is not None}
    runs = actor.runs if actor is not None else []
    stderr = current_native_stderr(actor)
    events = current_native_events(actor)
    replies = list(command_replies[-512:])
    raw = {"checkpoint":checkpoint,"deviceCommands":device_commands,"nativeRuns":runs,
        "currentNativeStderr":stderr,"currentNativeEvents":events,"nativeCommandReplies":replies}
    if stage_error is not None:
        raw["stageException"] = {"phase":phase,"exceptionType":type(stage_error).__name__,
            "exceptionText":str(stage_error)}
    encoded = (json.dumps(raw,indent=2)+"\n").encode()
    name = "private-command-receipt-" + checkpoint + ".json"
    path = output/name
    descriptor = os.open(path,os.O_WRONLY | os.O_CREAT | os.O_EXCL,0o600)
    with os.fdopen(descriptor,"wb") as handle:
        handle.write(encoded)
    def exact_exit(value):
        return value if type(value) is int else None
    return {"privateReceiptFile":name,"sha256":sha256(path),"mode":"0600",
        "currentNativeStderrLineCount":len(stderr["lines"]) if stderr is not None else 0,
        "currentNativeStderrGeneration":stderr["generation"] if stderr is not None else None,
        "currentNativeEventCount":len(events["events"]) if events is not None else 0,
        "nativeCommandReplyCount":len(replies),
        "stageExceptionClass":evidence_exception_class(stage_error) if stage_error is not None else None,
        "deviceCommandCounts":{role:len(commands) for role,commands in device_commands.items()},
        "deviceCommandExits":{role:[exact_exit(command.get("exit")) for command in commands]
            for role,commands in device_commands.items()},
        "nativeRunCount":len(runs),"nativeRuns":[{"generation":index+1,
            "commandExit":exact_exit(run.get("cleanup",{}).get("commandExit")),
            "forced":run.get("cleanup",{}).get("forced"),
            "independentCleanupElapsedMs":run.get("cleanup",{}).get("elapsedMs")}
            for index,run in enumerate(runs)]}


def persist_browser_start_observation(output, page, observer, checkpoint):
    """Existing owned-page fixture/CDP metadata survives a failed constructor."""
    raw = {"checkpoint":checkpoint,"cdp":observer.receipt() if observer else None}
    public = {"checkpoint":checkpoint,"fixtureObserved":False,"sockets":[]}
    try:
        reply = page.call("rtc_fixture_snapshot")
        raw["fixtureRpcReply"] = reply
        fixture = reply.get("result")
        if reply.get("ok") is True and isinstance(fixture,dict):
            public["fixtureObserved"] = True
            for socket in fixture.get("sockets",[])[:16]:
                public["sockets"].append({key:socket.get(key)
                    if type(socket.get(key)) in (int,float,bool) and
                        (type(socket.get(key)) is bool or (math.isfinite(socket[key]) and socket[key]>=0))
                    else None for key in ("createdAtMs","openedAtMs","closedAtMs","lastErrorAtMs",
                        "readyState","sentFrames","receivedFrames","sentBytes","receivedBytes",
                        "errorEvents","closeCode","closeWasClean")})
        else:
            public["fixtureFailureClass"] = "SdkReplyFailed"
    except Exception as error:
        raw["fixtureException"] = {"type":type(error).__name__,"text":str(error)}
        public["fixtureFailureClass"] = evidence_exception_class(error)
    cdp = raw["cdp"]
    public["cdpAvailable"] = isinstance(cdp,dict) and cdp.get("available") is True
    if isinstance(cdp,dict):
        events = cdp.get("events",[])
        public["cdpEventCounts"] = {kind:sum(event.get("event")==kind for event in events)
            for kind in ("socket-created","handshake-sent","handshake-response","socket-error","socket-closed")}
        public["cdpHttp101Count"] = sum(event.get("event")=="handshake-response" and event.get("status")==101 for event in events)
    else:
        public["cdpEventCounts"] = None
        public["cdpHttp101Count"] = None
    # Frame exchange and HTTP101 do not assert MQTT Connack/Suback success.
    filename = "private-browser-start-observation-"+checkpoint+".json"
    descriptor = os.open(output/filename,os.O_WRONLY|os.O_CREAT|os.O_EXCL,0o600)
    with os.fdopen(descriptor,"w") as handle:
        json.dump(raw,handle,indent=2);handle.write("\n")
    return {**public,"privateFile":filename,"sha256":sha256(output/filename),"mode":"0600"}



def is_admission_observation_event(event):
    if event.get("event") == "unified-control-ready":
        return event.get("purpose") == "admission-request" and event.get("purposeCode") == 2
    if event.get("event") in {"admission-accepted","admission-ended"}:
        return event.get("alpn") in {STRICT_ADMISSION_ALPN,STRICT_PRECODE_ALPN}
    return event.get("event") == "handler-error"


def persist_admission_observation(output, operation, reply, events, ordinal, checkpoint):
    """Exact failed RPC and finite existing A events stay in an exclusive private file."""
    raw_events = [entry for entry in events if is_admission_observation_event(entry["event"])]
    raw = {"operation":operation,"failedRpcReply":reply,"nativeAdmissionEvents":raw_events,
        "checkpoint":checkpoint,"correlation":"private Native stable IDs group events; RPC-to-group attribution may be unknown"}
    name = f"private-admission-observation-{ordinal:04d}-{checkpoint}.json"
    path = output/name
    descriptor = os.open(path,os.O_WRONLY | os.O_CREAT | os.O_EXCL,0o600)
    with os.fdopen(descriptor,"w") as handle:
        handle.write(json.dumps(raw,indent=2)+"\n")
    failure = reply.get("failure")
    # Match only fixed source-owned literals. No raw exception fragment is public.
    literals = {
        "owned admission ended":"OperatorAdmissionClose",
        "manual admission ended":"SdkAdmissionClose",
        "browser request ended or cancelled":"BrowserRequestClose",
        "unified restricted purpose rejected":"RestrictedPurposeRejected",
        "browser admission rejected":"BrowserGateRejected",
        "unified/ProtocolUnsupported":"ProtocolUnsupported",
        "unified/InvalidControl":"InvalidControl",
        "unified/IdentityOrAdmissionRejected":"IdentityOrAdmissionRejected",
        "temporarily unreachable; durable admission progress retained":"CallerTimeout",
    }
    matched = [tag for literal,tag in literals.items() if isinstance(failure,str) and literal in failure]
    groups = {}
    unpaired = 0
    for entry in raw_events:
        event = entry["event"]
        stable = event.get("stableId")
        if type(stable) is not int:
            unpaired += 1
            continue
        group = groups.setdefault((entry["generation"],stable),{})
        group[event["event"]] = event
    public_groups = []
    for (generation,_),group in groups.items():
        accepted,ended,common = (group.get(name,{}) for name in
            ("admission-accepted","admission-ended","unified-control-ready"))
        stage = ended.get("stage")
        public_groups.append({"generation":generation,
            "commonObserved":bool(common),"acceptedObserved":bool(accepted),"endedObserved":bool(ended),
            "acceptedAndEndedSameNativeM":bool(accepted) and bool(ended),
            "handlerSucceeded":ended.get("succeeded") if type(ended.get("succeeded")) is bool else None,
            "providerFailureClass":ended.get("providerFailureClass")
                if ended.get("providerFailureClass") in {"Rejected","PathUnavailable","Store","Node"} else None,
            "handlerStage":stage if stage in {"AwaitingApproval","Approved","PendingMaterials","Complete"} else None,
            "acceptedTiming":native_operation_facts(accepted,generation) if accepted else None,
            "endedTiming":native_operation_facts(ended,generation) if ended else None,
            "common":{key:common.get(key) for key in ("version","purposeCode","restricted",
                "ordinaryHandle","dataReleased","permitted","peerPermitted","stableUnchanged","bindingUnchanged")
                if type(common.get(key)) in (int,bool)}})
    return {"operation":operation,"checkpoint":checkpoint,"privateReceiptFile":name,
        "sha256":sha256(path),"mode":"0600","rpcOk":reply.get("ok") is True,
        "rpcCallerElapsedMs":reply.get("callerElapsedMs") if type(reply.get("callerElapsedMs")) in (int,float) else None,
        "failurePresent":isinstance(failure,str),"failureBuckets":matched or ["Unknown"],
        "nativeEventCount":len(raw_events),"nativeGroups":public_groups,"unpairedEventCount":unpaired,
        "rpcToNativeGroupAttributed":False}


def retain_admission_observation(report, output, operation, reply, events, ordinal, checkpoint):
    # Diagnostics must not replace the original failed RPC or change story eligibility.
    try:
        facts = persist_admission_observation(output,operation,reply,events,ordinal,checkpoint)
        report.setdefault("admissionFailureObservations",[]).append(facts)
    except Exception as error:
        report.setdefault("admissionObservationRetentionFailures",[]).append({
            "operation":operation,"checkpoint":checkpoint,"failureClass":type(error).__name__})


def sha256(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def bind_admission_old_wire_observation(path):
    """Read the rooted actual SDK observation; do not replay its SDK case."""
    receipt = json.loads(Path(path).read_text())
    source_path = Path(receipt["actualSdkSourceReceipt"])
    build_path = Path(receipt["actualSdkBuildReceipt"])
    require(sha256(source_path) == receipt.get("sourceReceiptSha256") == ADMISSION_BASELINE_SOURCE_SHA256,
            "admission baseline actual SDK receipt differs")
    require(sha256(build_path) == receipt.get("buildReceiptSha256") == ADMISSION_BASELINE_BUILD_SHA256,
            "admission baseline selected build receipt differs")
    source = json.loads(source_path.read_text())
    build = json.loads(build_path.read_text())
    accepted = [value for value in source["nativeEvents"] if value.get("event") == "admission-accepted"]
    alpns = [value.get("alpn") for value in accepted]
    require(len(accepted) == 2 and all(value.get("restricted") is True
            and value.get("businessHandle") is False for value in accepted),
            "actual restricted admission phase was not observed")
    require(source["savedMaterials"].get("stage") == "Complete"
            and source["savedMaterials"].get("authorization_saved") is True
            and source["savedMaterials"].get("secret_saved") is True
            and source.get("identitiesReopened") == {"browser": True, "native": True},
            "actual saved materials and reopened identities are missing")
    require(build.get("exit") == 0 and build.get("sourceStable") is True
            and build.get("allSelectedInputsMatched") is True, "actual baseline build binding differs")
    require(alpns == receipt.get("actualNegotiatedAlpns")
            and all(alpn == "miopunch/admission-request/1" for alpn in alpns)
            and receipt.get("expectedStrictPurposeAlpn") == STRICT_ADMISSION_ALPN
            and receipt.get("assertionExit") == 1 and receipt.get("newSdkRunPerformed") is False,
            "actual strict-purpose assertion baseline differs")
    return {"scope": "actual restricted admission succeeded; strict new-purpose assertion RED",
            "observationReceiptSha256": sha256(path), "sourceReceiptSha256": sha256(source_path),
            "buildReceiptSha256": sha256(build_path), "assertionExit": 1,
            "actualNegotiatedAlpns": alpns, "expectedStrictPurposeAlpn": STRICT_ADMISSION_ALPN,
            "savedMaterialsComplete": True, "bothIdentitiesReopened": True,
            "parentMemberCasePassed": source.get("passed") is True,
            "parentMemberRoleFailureDoesNotInvalidateAdmission": True,
            "newSdkRunPerformed": False, "replicaOldWireRedCaptured": False,
            "newMuc1Qualified": False}


def purpose_wire_facts(alpn, coordinator, purpose, *, precode=False):
    """Whitelist actual diagnostic fields. Missing fields remain unknown."""
    expected = {"admission-request": (STRICT_PRECODE_ALPN if precode else STRICT_ADMISSION_ALPN, 2),
                "replica": (STRICT_REPLICA_ALPN, 3)}[purpose]
    value = coordinator if isinstance(coordinator, dict) else {}
    def boolean(name):
        return value.get(name) if type(value.get(name)) is bool else None
    def integer(name):
        return value.get(name) if type(value.get(name)) is int else None
    known_alpns = {STRICT_ADMISSION_ALPN, STRICT_PRECODE_ALPN, STRICT_REPLICA_ALPN,
                   "miopunch/admission-request/1", "miopunch/precode-request/1",
                   "miopunch/control-replica/1", "miopunch/control-replica/rtc-path/3"}
    event = value.get("event")
    wanted_event = "unified-control-ready" if purpose == "admission-request" else "unified-path-ready"
    facts = {"actualAlpn": alpn if isinstance(alpn, str) and alpn in known_alpns else None, "expectedAlpn": expected[0],
             "strictAlpnMatched": alpn == expected[0], "observedVersion": integer("version"),
             "observedPurposeCode": integer("purposeCode"),
             "matchingCommonPurposeObserved": event == wanted_event and integer("version") == 1
                 and integer("purposeCode") == expected[1] and value.get("purpose") == purpose,
             "restricted": boolean("restricted"), "dataReleased": boolean("dataReleased"),
             "ordinaryHandle": boolean("ordinaryHandle"), "freshAck": boolean("freshAck"),
             "freshAckFieldPresent": "freshAck" in value, "freshAckExplicitNull": "freshAck" in value and value["freshAck"] is None,
             "stableUnchanged": boolean("stableUnchanged"),
             "bindingUnchanged": boolean("bindingUnchanged")}
    return facts


def require_admission_wire(diagnostics, native_accepted, *, precode=False):
    facts = purpose_wire_facts(diagnostics.get("request", {}).get("alpn"),
        diagnostics.get("gate", {}).get("coordinator"), "admission-request", precode=precode)
    require(facts["strictAlpnMatched"] and facts["matchingCommonPurposeObserved"],
            "actual admission still uses old ALPN or lacks strict common purpose dialogue")
    require(facts["restricted"] is True and facts["ordinaryHandle"] is False
            and facts["dataReleased"] is False and facts["stableUnchanged"] is True
            and facts["bindingUnchanged"] is True and facts["freshAckExplicitNull"],
            "restricted admission dialogue or original binding proof differs")
    require(native_accepted.get("alpn") == facts["expectedAlpn"]
            and native_accepted.get("restricted") is True
            and native_accepted.get("businessHandle") is False,
            "native actual admission scope is not the same restricted purpose")
    return facts


def require_replica_wire(diagnostics, native_finished):
    before, after = diagnostics.get("before", {}), diagnostics.get("after", {})
    facts = purpose_wire_facts(before.get("alpn"), before.get("bootstrap", {}).get("coordinator"), "replica")
    require(facts["strictAlpnMatched"] and facts["matchingCommonPurposeObserved"]
            and facts["freshAck"] is True, "actual replica lacks strict common purpose and original-M fresh ACK")
    require(after.get("alpn") == STRICT_REPLICA_ALPN and before.get("currentPermission") is True
            and after.get("currentPermission") is True and diagnostics.get("stableUnchanged") is True
            and diagnostics.get("bindingUnchanged") is True, "replica current member/binding checks differ")
    require(native_finished.get("alpn") == STRICT_REPLICA_ALPN
            and native_finished.get("memberAuthorizedBefore") is True
            and native_finished.get("durableReceiptAccepted") is True
            and native_finished.get("stableUnchanged") is True
            and native_finished.get("bindingUnchanged") is True,
            "native replica lacks actual current member, original M or durable receipt proof")
    return facts


def material_facts(material):
    """Publish explicit fields, never recursively dump SDK/private replies."""
    require(type(material.get("authorization_saved")) is bool, "authorization observation missing")
    require(type(material.get("secret_saved")) is bool, "secret observation missing")
    require(material.get("stage") in ("AwaitingApproval", "PendingMaterials", "Complete"), "unrecognized material stage")
    return {"stage": material["stage"], "authorizationSaved": material["authorization_saved"],
            "secretSaved": material["secret_saved"]}


def same_grant(before, after):
    return all(isinstance(before.get(name),str) and bool(before[name]) and before[name] == after.get(name)
               for name in ("id", "request", "authorization"))


def require_complete(material):
    facts = material_facts(material)
    require(facts == {"stage": "Complete", "authorizationSaved": True, "secretSaved": True},
            "durable Complete materials were not observed")
    return facts


def require_replica(outcome, target, before, after):
    require(target not in before.get("records", []), "target was already in browser store")
    require(target in after.get("records", []), "target absent from verified saved browser store")
    for name in ("scope", "saved", "applied"):
        require(isinstance(outcome.get(name), list) and target in outcome[name], "nonempty replica scope/save/apply proof missing")
    require(outcome.get("complete") is True, "replica outcome is incomplete")
    return {"targetAbsentBefore": True, "targetVerifiedSavedAfter": True,
            "scopeCount": len(outcome["scope"]), "savedCount": len(outcome["saved"]),
            "appliedCount": len(outcome["applied"]), "complete": True}


def fixture_facts(page, predicate, timeout=3):
    deadline = time.monotonic() + timeout
    with page.condition:
        while True:
            reply = page.responses.get("mio83-admission-fixture")
            if reply and reply.get("ok") is True and predicate(reply.get("result", {})):
                return reply["result"]
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError("actual IndexedDB boundary receipt unavailable")
            page.condition.wait(remaining)


def prelude():
    # This is a pure export call in Node. It does not run the browser fixture.
    rtc_fixture = ROOT / "browser-probe/harness/rtc-capability-fixture.mjs"
    script = ("import {admissionCommitPrelude} from " + json.dumps(FIXTURE.as_uri())
        + ";import {rtcCapabilityPrelude} from " + json.dumps(rtc_fixture.as_uri())
        + ";process.stdout.write(rtcCapabilityPrelude('observe')+'\\n'+admissionCommitPrelude());")
    return subprocess.run(["rtk", "proxy", "node", "--input-type=module", "-e", script],
                          check=True, capture_output=True, text=True).stdout


def development_checks():
    sentinel = "PRIVATE_MATERIAL_DO_NOT_EMIT"
    before = {"id": sentinel, "request": sentinel, "authorization": sentinel,
              "stage": "PendingMaterials", "authorization_saved": True, "secret_saved": False,
              "secret": sentinel, "records": []}
    require(sentinel not in json.dumps(material_facts(before)), "material privacy boundary failed")
    require(same_grant(before, dict(before)), "matching durable namespace rejected")
    require(not same_grant(before, {**before, "request": "different"}), "different namespace accepted")
    rejected = 0
    for value in ({**before, "stage": "Complete"}, {**before, "authorization_saved": 1}):
        try:
            require_complete(value)
        except RuntimeError:
            rejected += 1
    require(rejected == 2, "early Complete or unknown material facts accepted")
    target = "a" * 64
    after = {**before, "records": [target]}
    valid = {"complete": True, "scope": [target], "saved": [target], "applied": [target]}
    require_replica(valid, target, before, after)
    try:
        require_replica({**valid, "saved": []}, target, before, after)
    except RuntimeError:
        pass
    else:
        raise RuntimeError("unsaved replica accepted")
    # Simulation verifies wrapper ownership/privacy and event ordering, not
    # real IndexedDB durability or SDK/wire acceptance.
    node = r'''
import assert from 'node:assert/strict';
import vm from 'node:vm';
import {installAdmissionCommitFixture} from FIXTURE_URI;
const posts=[];
class Transaction {
  constructor(){this.db={name:'miopunch-browser-network'};this.mode='readwrite';this.listeners={};this.abortCalls=0;}
  addEventListener(name,handler){(this.listeners[name]??=[]).push(handler);}
  dispatch(name){for(const handler of this.listeners[name]??[])handler();}
  abort(){this.abortCalls++;}
}
class Store {constructor(tx){this.name='node';this.transaction=tx;this.calls=0;}put(){this.calls++;return 'request';}}
const original=Store.prototype.put;
const context={URL,location:{href:'http://127.0.0.1:12345/?actor=owned'},IDBObjectStore:Store,
  addEventListener(){},fetch(path,args){posts.push(JSON.parse(args.body));return Promise.resolve({ok:true});}};
vm.createContext(context);vm.runInContext('('+installAdmissionCommitFixture.toString()+')()',context);
const secret='PRIVATE_SNAPSHOT_SECRET';
let tx=new Transaction(),store=new Store(tx);
store.put(JSON.stringify({authorization:secret,secret:null}), 'state');
assert.equal(context.__mio83AdmissionFixture.snapshot().authorizationOnlyCommitted,false);
tx.dispatch('complete');
assert.equal(context.__mio83AdmissionFixture.snapshot().authorizationOnlyCommitted,true);
tx=new Transaction();store=new Store(tx);
store.put(JSON.stringify({authorization:secret,secret}), 'state');
assert.equal(tx.abortCalls,1);assert.equal(store.calls,0);
assert.equal(context.__mio83AdmissionFixture.snapshot().secretAbortObserved,null);
tx.dispatch('abort');assert.equal(context.__mio83AdmissionFixture.snapshot().secretAbortObserved,true);
tx=new Transaction();store=new Store(tx);store.put(JSON.stringify({authorization:secret,secret}), 'state');
assert.equal(store.calls,1);assert.equal(tx.abortCalls,0);tx.dispatch('complete');
assert.equal(context.__mio83AdmissionFixture.snapshot().transactions.at(-1).committed,true);
assert.equal(JSON.stringify(posts).includes(secret),false);
context.__mio83AdmissionFixture.restore();assert.equal(Store.prototype.put,original);
assert.throws(()=>vm.runInContext('('+installAdmissionCommitFixture.toString()+')()',context));
process.stdout.write(JSON.stringify({privacy:true,oneShotAbort:true,eventOrdering:true,restored:true}));
'''.replace("FIXTURE_URI", json.dumps(FIXTURE.as_uri()))
    executed = subprocess.run(["rtk", "proxy", "node", "--input-type=module"], input=node,
                              capture_output=True, text=True, timeout=10)
    require(executed.returncode == 0, "offline fixture simulation failed: " + executed.stderr)
    return {"scope": "offline privacy/phase/transaction-event simulation only", "sdkRun": False,
            "deviceOperation": False, "publicNetworkOperation": False,
            "nodeExit": executed.returncode, "nodeFacts": json.loads(executed.stdout),
            "materialPrivacy": True, "sameGrantBinding": True, "earlyCompleteRejected": True,
            "unsavedReplicaRejected": True}


def validate_sdk_build_sources(receipt):
    """Keep broad captures historical; prove every declared delta unselected."""
    before = json.loads((receipt.parent / "sources-before.json").read_text())
    after = json.loads((receipt.parent / "sources-after.json").read_text())
    require(before == after, "SDK build source snapshots differ")
    build = json.loads(receipt.read_text())
    digest = hashlib.sha256(json.dumps(after,sort_keys=True).encode()).hexdigest()
    require(build.get("sourceDigestBefore") == digest and build.get("sourceDigestAfter") == digest,
            "SDK captured source snapshot digest differs")
    selected = json.loads((receipt.parent / "selected-dependency-inputs.json").read_text())
    require(isinstance(selected.get("inputs"),dict) and selected["inputs"], "actual compiled inputs missing")
    require(all(Path(name).is_file() and sha256(name) == expected
                for name,expected in selected["inputs"].items()), "actual compiled SDK input changed")
    runtime_sources = {"scripts/check_unified_admission.py","scripts/host_native_runtime.py"}
    normalized_compiler_inputs = {str(Path(value).resolve()) for value in selected["inputs"]}
    runtime_unselected = {}
    for name in runtime_sources:
        path = ROOT/name
        require(str(path.resolve()) not in normalized_compiler_inputs,
                "runtime Python source unexpectedly selected by SDK compiler: "+name)
        runtime_unselected[name] = {"currentSha256":sha256(path),"actualCompilerInput":False}
    wasm_graph = any(command.get("label") == "wasm" and "--target" in command.get("command",[])
        and command["command"][command["command"].index("--target")+1] == "wasm32-unknown-unknown"
        for command in build.get("commands",[]))
    wasm_native_source_scopes = {
        "src/bin/unified_admission_story.rs":"Native CLI operator leaf, absent from the actual WASM compiler inputs",
        "src/network_maintenance.rs":"Native maintenance module, absent from the actual WASM compiler inputs",
        "src/path_selection.rs":"Native path-selection module, absent from the actual WASM compiler inputs",
        "src/bin/browser_wss_member_native.rs":"Native member fixture leaf, absent from the actual WASM compiler inputs",
        "src/mqtt_transport.rs":"Native MQTT driver module, absent from the actual WASM compiler inputs",
    }
    other_platform_sources = set(wasm_native_source_scopes) if wasm_graph else set()
    native_graph = build.get("nativePlatform") in {"host","android"} and not wasm_graph
    if native_graph:
        other_platform_sources.add("browser-probe/src/replica.rs")
    other_platform_unselected = {}
    for name in other_platform_sources:
        path = ROOT/name
        require(str(path.resolve()) not in normalized_compiler_inputs,
                "declared other-platform source unexpectedly selected by SDK compiler: "+name)
        other_platform_unselected[name] = {"currentSha256":sha256(path),"actualCompilerInput":False,
            "scope":wasm_native_source_scopes[name]
                if wasm_graph else "Browser Replica leaf, absent from the actual Native compiler inputs"}
    independent_sources = {"src/bin/unified_maintained_story.rs"}
    independent_unselected = {}
    for name in independent_sources:
        path = ROOT/name
        require(str(path.resolve()) not in normalized_compiler_inputs,
                "independently owned executable unexpectedly selected by SDK compiler: "+name)
        independent_unselected[name] = {"currentSha256":sha256(path) if path.is_file() else None,
            "present":path.is_file(),"actualCompilerInput":False,
            "scope":"independently owned M77 executable, not selected by this compiled SDK graph"}
    copies = selected.get("copiedRepositoryInputs",{})
    require(copies and all(sha256(receipt.parent/"selected-source-copies"/name) == expected
                          for name,expected in copies.items()), "captured selected SDK source copy differs")
    deltas = {}
    for name,expected in after.items():
        path = Path(name.removeprefix("external:")) if name.startswith("external:") else ROOT/name
        actual = sha256(path) if path.is_file() else None
        if actual == expected:
            continue
        require(name in runtime_sources | other_platform_sources | independent_sources and str(path.resolve()) not in normalized_compiler_inputs,
                "SDK or undeclared runtime input changed: "+name)
        deltas[name] = {"capturedSha256":expected,"currentSha256":actual,
            "actualCompilerInput":False,"reason":"separate Python runtime source; not selected by SDK compiler"
                if name in runtime_sources else (independent_unselected[name]["scope"]
                    if name in independent_sources else other_platform_unselected[name]["scope"])}
    return {"actualSelectedInputCount":len(selected["inputs"]),"allCompiledSourcesMatch":True,
        "capturedBroadSourcesEqualCurrent":not deltas,"unselectedRuntimeControllerDeltas":deltas,
        "inactiveCurrentSourceDeltas":{},"runtimeSourcesProvenUnselected":runtime_unselected,
        "otherPlatformSourcesProvenUnselected":other_platform_unselected,
        "independentlyOwnedSourcesProvenUnselected":independent_unselected,
        "snapshotSha256":sha256(receipt.parent/"sources-after.json")}


def checked(reply):
    require(reply.get("ok") is True, "browser SDK operation failed")
    return reply["result"]


def require_tls_probe_reply(reply):
    elapsed = reply.get("callerElapsedMs")
    require(reply.get("ok") is False and reply.get("failure") == "connection lost"
            and reply.get("result") is None, "TLS claim probe lacked the expected failed no-material reply")
    require(type(elapsed) in (int,float) and math.isfinite(elapsed) and 0 <= elapsed < 12000,
            "TLS claim probe caller timing is missing or exhausted")
    return reply


def require_tls_claim_rejection(reply, events, prior_connections, generation, before, after):
    require_tls_probe_reply(reply)
    groups = {}
    for entry in events:
        event = entry["event"]
        stable = event.get("stableId")
        require(type(stable) is int and entry.get("generation") == generation,
                "TLS claim probe Native event identity or generation is unknown")
        key = (generation,stable)
        if key in prior_connections:
            continue
        require(event.get("event") in {"unified-control-ready","admission-accepted","admission-ended"},
                "TLS claim probe Native event type is unknown")
        group = groups.setdefault(key,{})
        require(event["event"] not in group, "TLS claim probe Native event is ambiguous")
        group[event["event"]] = event
    require(len(groups) == 1, "TLS claim probe lacks one fresh Native A")
    group = next(iter(groups.values()))
    accepted,ended,common = (group.get(name,{}) for name in
        ("admission-accepted","admission-ended","unified-control-ready"))
    facts = purpose_wire_facts(accepted.get("alpn"),common,"admission-request")
    require(facts["strictAlpnMatched"] and facts["matchingCommonPurposeObserved"]
            and facts["restricted"] is True and facts["ordinaryHandle"] is False
            and facts["dataReleased"] is False and facts["stableUnchanged"] is True
            and facts["bindingUnchanged"] is True and facts["freshAckExplicitNull"],
            "TLS claim probe lacks strict restricted original-M purpose evidence")
    require(type(common.get("permitted")) is int and common["permitted"] == 0
            and type(common.get("peerPermitted")) is int and common["peerPermitted"] == 0
            and accepted.get("restricted") is True and accepted.get("businessHandle") is False,
            "TLS claim probe Native A released data permission")
    require(ended.get("alpn") == STRICT_ADMISSION_ALPN and ended.get("succeeded") is False
            and ended.get("stage") is None and ended.get("providerFailureClass") == "Rejected"
            and ended.get("operatorRetirementReason") == "RejectedWithoutWait",
            "TLS claim probe lacks an actual canonical provider rejection")
    accepted_ms,ended_ms = accepted.get("callerElapsedMs"),ended.get("callerElapsedMs")
    require(accepted.get("callerDeadlinePresent") is True
            and type(accepted.get("callerBudgetMs")) is int and accepted["callerBudgetMs"] == 12000
            and type(ended.get("callerBudgetMs")) is int and ended["callerBudgetMs"] == 12000
            and type(accepted_ms) is int and type(ended_ms) is int
            and 0 <= accepted_ms <= ended_ms < 12000,
            "TLS claim probe Native original caller timing is missing or exhausted")
    require(type(before.get("requests")) is list and type(after.get("requests")) is list
            and type(before.get("appliedCount")) is int and before["appliedCount"] >= 0
            and type(after.get("appliedCount")) is int
            and after["requests"] == before["requests"] and after["appliedCount"] == before["appliedCount"],
            "wrong TLS claim changed admission authorization or lacks exact ledger evidence")
    return {"tlsClaimRejected":True,"noApprovedMaterial":True,"noAuthorizationMutation":True,
        "providerFailureClass":"Rejected","oneFreshRestrictedNativeAInProbeInterval":True,
        "rpcToNativeGroupAttributed":False,"sdkCallerElapsedMs":reply["callerElapsedMs"],
        "nativeProviderCallerElapsedMs":ended_ms,"nativeCallerBudgetMs":12000}


def require_old_grant_rejection(reply, events, prior_connections, generation, before, after):
    # The same restricted canonical failure proof applies to an invalid grant.
    # Project generic facts: this request does not claim a wrong TLS identity.
    facts = require_tls_claim_rejection(reply, events, prior_connections, generation, before, after)
    return {"canonicalRejected":facts["providerFailureClass"] == "Rejected",
        "currentGeneration":generation,"noApprovedMaterial":facts["noApprovedMaterial"],
        "noAuthorizationMutation":facts["noAuthorizationMutation"],
        "oneFreshRestrictedNativeAInRequestInterval":facts["oneFreshRestrictedNativeAInProbeInterval"],
        "rpcToNativeGroupAttributed":False,"sdkCallerElapsedMs":facts["sdkCallerElapsedMs"],
        "nativeProviderCallerElapsedMs":facts["nativeProviderCallerElapsedMs"],
        "nativeCallerBudgetMs":facts["nativeCallerBudgetMs"]}


def require_unknown_data_rejection(reply):
    require(reply.get("ok") is False and reply.get("result") is None
            and reply.get("failure") in {"admission rejected or materials incomplete","identity or admission rejected"},
            "unknown applicant lacks an explicit SDK data permission rejection")
    return reply


def run(args):
    # Device construction is intentionally confined to explicit execution.
    from android_pair_runtime import BrowserPage, Device, NativeActor, browser_cleanup_succeeded
    from check_unified_browser_native import close_native, native_cleanup_succeeded, validate_build_sources, redacted
    from host_native_runtime import HostNativeActor, host_native_cleanup_succeeded
    native_cleanup_check = host_native_cleanup_succeeded if args.native_platform == "host" else native_cleanup_succeeded
    def publish_sdk_facts(value):
        # Raw original-M pointers are used only for private event pairing.
        def strip_local_ids(value):
            if isinstance(value,list):
                return [strip_local_ids(child) for child in value]
            if isinstance(value,dict):
                return {key:strip_local_ids(child) for key,child in value.items()
                    if key.replace("_","").lower() not in {"stableid","controlstableid"}}
            return value
        return strip_local_ids(redacted(value))

    require(not args.output.exists(), "output exists; preserve original failure")
    native_build = json.loads(args.build_receipt.read_text())
    wasm_build = json.loads(args.wasm_build_receipt.read_text())
    require(native_build.get("qualifiedForFrozenRun") is True and wasm_build.get("qualifiedForFrozenRun") is True,
            "successful frozen-source builds are required")
    require(native_build.get("nativePlatform","android") == args.native_platform,
            "native artifact platform differs from the selected actor")
    native_binding = validate_sdk_build_sources(args.build_receipt)
    browser_binding = validate_sdk_build_sources(args.wasm_build_receipt)
    # This fixture is newly compiled; no inactive-source exceptions are valid.
    require(not native_binding["inactiveCurrentSourceDeltas"], "native source differs from build")
    require(not browser_binding["inactiveCurrentSourceDeltas"], "browser source differs from build")
    deployed = native_build["deployment"]["unified_admission_story"]
    binary = Path(deployed["deployedPath"])
    require(sha256(binary) == deployed["deployedSha256"]
            and sha256(deployed["compiledPath"]) == deployed["compiledSha256"]
            and deployed["transformation"].get("exit") == 0, "native artifact transformation chain differs")
    for name, digest in wasm_build["artifacts"].items():
        require(sha256(args.package/name) == digest, "browser package differs from build")
    require(args.require_strict_purpose, "full story requires strict purpose checks")
    precode = json.loads(args.precode_receipt.read_text())
    require(precode.get("exit") == 0 and precode.get("strictPurposeObservedOnBothEndpoints") is True
            and precode.get("selectedSourcesStable") is True and precode.get("artifactStable") is True
            and precode.get("processGroupAbsentImmediate") is True
            and precode.get("processGroupAbsentLate") is True and precode.get("forced") is False,
            "affected precode short regression is not qualified")
    facility = json.loads(args.facility.read_text())
    browser_profile = json.loads(args.browser_profile.read_text())
    args.output.mkdir(mode=0o700, parents=True)
    report = {"scope": "actual public SDK manual admission, interrupted durable save, identity reopen, offline nonempty Replica and bidirectional business",
              "newWireQualified": False, "physicalWanQualified": False, "passed": False,
              "preparationPassed": False, "startedUtc": time.strftime("%Y-%m-%dT%H:%M:%SZ",time.gmtime()),
              "sources": {"runner":sha256(__file__),"commitFixture":sha256(FIXTURE),
                  "rtcObserver":sha256(ROOT/"browser-probe/harness/rtc-capability-fixture.mjs"),
                  "page":sha256(ROOT/"browser-probe/harness/android-pair-page.js")},
              "artifacts": {"native":deployed["deployedSha256"],"wasm":wasm_build["artifacts"]},
              "buildReceipts": {"native":sha256(args.build_receipt),"wasm":sha256(args.wasm_build_receipt)},
              "compiledSourceBindings": {"native":native_binding,"browser":browser_binding},
              "stages": [], "remainingQualification": ["full public SDK story"],
              "affectedPrecodeRegression": {"receiptSha256":sha256(args.precode_receipt),"actualExit":precode["exit"],
                  "newRestrictedPurpose":True,"sameGrantResume":True,"onceOnlyUsage":True},
              "resourceChoice": {"permittedRelay":args.relay,"rtcEnabled":not args.rtc_disabled},
              "command": ["rtk","proxy","python3",str(Path(__file__).resolve()),*sys.argv[1:]],
              "selectedProfiles": {"facilitySha256":sha256(args.facility),"nativeIceSha256":sha256(args.native_profile),
                  "browserIceSha256":sha256(args.browser_profile)},
              "sdkCallerBudgetMs":12000}
    report["nativePlatform"] = args.native_platform
    report["environmentQualification"] = {"hostWarp":None,"nativeOsRoute":None,
        "scope":"selected Native platform and allowed SDK path; physical OS route/VPN/NAT facts are not inferred"}
    if args.admission_old_wire_observation:
        report["admissionOldWireBaseline"] = bind_admission_old_wire_observation(args.admission_old_wire_observation)
    actor = page = native_device = browser_device = None
    observer = None
    browser_start_observation_count = 0
    current_phase = "native-setup"
    stage_depth = 0
    native_timings_seen = set()
    observed_admission_events = []
    admission_events_seen = set()
    failed_admission_rpcs = []
    failed_other_sdk_rpc_count = 0
    stage_exception_observation_attempted = False
    native_command_replies = []

    def save():
        (args.output/"receipt.json").write_text(json.dumps(report,indent=2)+"\n")

    def stage(name, operation):
        nonlocal current_phase, stage_depth, stage_exception_observation_attempted
        current_phase = name
        stage_depth += 1
        started = time.monotonic()
        try:
            result = operation()
            if isinstance(result,dict):
                keep_admission_event(result)
            report["stages"].append({"name":name,"ok":True,"callerMs":round((time.monotonic()-started)*1000,3)})
            save(); return result
        except Exception as error:
            # SDK error strings may include private peer/material values.
            remember_first_failure(report,error,name)
            if not stage_exception_observation_attempted:
                stage_exception_observation_attempted = True
                try:
                    report["stageExceptionEvidence"] = persist_command_evidence(args.output,
                        {"native":native_device,"browser":browser_device},actor,"stage-error",
                        stage_error=error,phase=name,command_replies=native_command_replies)
                except Exception as retention_error:
                    report["stageExceptionRetentionFailure"] = {
                        "failureClass":evidence_exception_class(retention_error)}
            report["stages"].append({"name":name,"ok":False,"failureClass":type(error).__name__,
                                     "callerMs":round((time.monotonic()-started)*1000,3)})
            save(); raise
        finally:
            stage_depth -= 1

    def command(operation, expected, **values):
        nonlocal current_phase
        if stage_depth == 0:
            current_phase = "native-command:" + operation
        return native_command(actor,operation,expected,native_command_replies,**values)

    def sdk(operation, values=None):
        nonlocal current_phase, failed_other_sdk_rpc_count, browser_start_observation_count
        if stage_depth == 0:
            current_phase = "sdk:" + operation
        try:
            reply = page.call(operation, values)
        except Exception as error:
            if operation == "start":
                remember_first_failure(report,error,current_phase)
                save()
                browser_start_observation_count += 1
                try:
                    facts = persist_browser_start_observation(args.output,page,observer,
                        "failed-start-"+str(browser_start_observation_count))
                    report.setdefault("browserStartObservations",[]).append(facts)
                except Exception as retention_error:
                    report.setdefault("browserStartObservationRetentionFailures",[]).append({
                        "failureClass":evidence_exception_class(retention_error)})
                save()
            raise
        report.setdefault("sdkCalls",[]).append({"operation":operation,"ok":reply.get("ok") is True,
            "callerElapsedMs":reply.get("callerElapsedMs")})
        if reply.get("ok") is False and operation in {"request_admission","request_precode","admission_identity_probe","complete_admission"}:
            collect_admission_events()
            ordinal = len(failed_admission_rpcs)+1
            failed_admission_rpcs.append((operation,reply,ordinal))
            retain_admission_observation(report,args.output,operation,reply,
                observed_admission_events,ordinal,"rpc-return")
        elif reply.get("ok") is False:
            failed_other_sdk_rpc_count += 1
            # Earlier A groups cannot identify an unrelated SDK RPC. Retain
            # its exact reply with no claimed Native event correlation.
            retain_admission_observation(report,args.output,operation,reply,
                [],failed_other_sdk_rpc_count,"sdk-rpc-return")
        if operation == "start" and reply.get("ok") is False:
            remember_first_failure(report,EvidenceCheckFailure("browser SDK operation failed"),current_phase)
            save()
            browser_start_observation_count += 1
            try:
                facts = persist_browser_start_observation(args.output,page,observer,
                    "failed-start-"+str(browser_start_observation_count))
                report.setdefault("browserStartObservations",[]).append(facts)
            except Exception as error:
                report.setdefault("browserStartObservationRetentionFailures",[]).append({
                    "failureClass":evidence_exception_class(error)})
        save()
        return reply

    def observe_browser_carriers(checkpoint):
        value = {"checkpoint":checkpoint}
        public = {"checkpoint":checkpoint,"observed":False,"carriers":[]}
        try:
            reply = page.call('path_diagnostics')
            value["reply"] = reply
            diagnostics = reply.get('result')
            if reply.get('ok') is True and isinstance(diagnostics,dict):
                public['observed'] = True
                for carrier in diagnostics.get('carriers',[])[:8]:
                    traffic = carrier.get('traffic',{})
                    integer_keys = ('sent_packets','sent_bytes','received_packets','received_bytes',
                        'rejected_packets','sent_frames','sent_frame_bytes','received_frames',
                        'received_frame_bytes','connack','subscribe_id','suback_id')
                    counts = {key:traffic.get(key) if type(traffic.get(key)) is int and traffic[key]>=0
                        else None for key in integer_keys}
                    statuses = {key:carrier.get(key) if type(carrier.get(key)) is bool else None
                        for key in ('ready','closed')}
                    statuses['failurePresent'] = isinstance(traffic.get('failure'),str)
                    statuses['subackCodes'] = traffic.get('suback_codes')[:8] if isinstance(traffic.get('suback_codes'),list)                         and all(type(code) is int and 0<=code<=255 for code in traffic['suback_codes']) else None
                    public['carriers'].append({**statuses,'traffic':counts,
                        **{key:carrier.get(key) if type(carrier.get(key)) is int and carrier[key]>=0 else None
                           for key in ('queued_packets','pending_bytes','buffered_bytes')}})
            else:
                public['failureClass'] = 'SdkReplyFailed'
        except Exception as error:
            value['exception'] = {'type':type(error).__name__,'text':str(error)}
            public['failureClass'] = evidence_exception_class(error)
        # Save exact diagnostics before a failed request is checked. The getter
        # never replaces the original request's DTO, phase or first failure.
        filename = 'private-browser-path-diagnostics-'+checkpoint+'.json'
        descriptor = os.open(args.output/filename,os.O_WRONLY|os.O_CREAT|os.O_EXCL,0o600)
        with os.fdopen(descriptor,'w') as handle:
            json.dump(value,handle,indent=2);handle.write('\n')
        public['privateFile'] = filename
        public['sha256'] = sha256(args.output/filename)
        report.setdefault('browserCarrierObservations',[]).append(public)
        save()

    def stop_browser():
        value = checked(sdk("stop"))
        require(value.get("cleanupSucceeded") is True and value.get("networkClosed") is True
                and value.get("networkFreed") is True and value.get("ownedMemberHandles") == 0,
                "browser SDK retirement incomplete")
        return value

    def keep_native_timing(event):
        keep_admission_event(event)
        generation = len(actor.runs)
        key = (generation,id(event))
        facts = native_operation_facts(event,generation)
        if facts is not None:
            if key not in native_timings_seen:
                report.setdefault("nativeOperations",[]).append(facts)
                native_timings_seen.add(key)

    def keep_admission_event(event):
        if actor is None or not is_admission_observation_event(event):
            return
        key = (len(actor.runs),id(event))
        if key not in admission_events_seen:
            observed_admission_events.append({"generation":len(actor.runs),"event":event})
            admission_events_seen.add(key)

    def collect_admission_events():
        if actor is not None:
            with actor.condition:
                events = list(actor.events)
            for event in events:
                keep_admission_event(event)

    def snapshot_execution(checkpoint):
        # Called after generation cleanup, before start() clears its event queue.
        if actor is not None:
            for event in actor.events:
                keep_native_timing(event)
        facts = persist_command_evidence(args.output,
            {"native":native_device,"browser":browser_device},actor,checkpoint,
            command_replies=native_command_replies)
        report.setdefault("executionEvidence",[]).append(facts)
        save()

    try:
        if args.native_platform == "host":
            actor = stage("owned-host-native-actor",HostNativeActor)
        else:
            native_device = stage("native-device-selection",lambda:Device("TB322FC","network"))
            actor = stage("owned-native-actor",lambda:NativeActor(native_device))
        actor.deploy(binary,"admission-story",executable=True)
        facility_remote = actor.deploy(args.facility,"facility.json")
        profile_remote = actor.deploy(args.native_profile,"ice-profile.json")
        data_dir = actor.directory+"/saved-provider"
        native_args = [data_dir,facility_remote,profile_remote,"--allow-unqualified-preparation"]
        if args.relay:
            native_args += ["--relay",args.relay]
        if args.rtc_disabled:
            native_args += ["--rtc-disabled"]
        actor.start(native_args)
        ready = stage("native-public-sdk-start",lambda:actor.wait_event("ready"))
        require(ready.get("normalPublicTrust") is True and ready.get("newWireQualified") is False, "native preparation contract differs")
        browser_device = stage("browser-device-selection",lambda:Device("Pixel_6a","usb"))
        page = BrowserPage(browser_device,args.package,prelude=prelude(),environment_rtc_probe=False)
        stage("owned-browser-page",page.start)
        from check_browser_relay_capability import OwnedCdpObserver
        observer = OwnedCdpObserver(page)
        observer.start()
        def configuration(current, *, data=False):
            # A uses MQTT even if a data relay is permitted later. Reopening the
            # same SDK store changes resources, never its identity or authorization.
            return {"relays":[args.relay] if data and args.relay else [],"rtc_enabled":not args.rtc_disabled,
                    "disabled":[],"irc":[],"mqtt":[facility],"rtc_ice":browser_profile,
                    "contact_routes":[{"contact":current["id"],"facility":facility["logical_id"],"address":current["route"]}]}

        def native_common(purpose, stable):
            nonlocal current_phase
            current_phase = "native-common:" + purpose
            event = actor.wait_event(lambda event: event.get("event") in ("unified-control-ready","unified-path-ready")
                and event.get("purpose") == purpose and event.get("stableId") == stable)
            keep_admission_event(event)
            require(event.get("version") == 1 and event.get("stableUnchanged") is True
                    and event.get("bindingUnchanged") is True, "native original purpose dialogue differs")
            return event
        first = stage("independent-browser-start",lambda:checked(sdk("start",{"configuration":configuration(ready)})))
        browser_id = first["id"]
        require(browser_id != ready["id"], "browser identity must be independent")
        observe_browser_carriers("before-first-request")
        first_request_reply = sdk("request_admission",{"invitation":ready["invitation"]})
        if first_request_reply.get("ok") is not True:
            remember_first_failure(report,EvidenceCheckFailure("browser SDK operation failed"),current_phase)
            save()
        observe_browser_carriers("after-first-request")
        require(checked(first_request_reply) == "AwaitingApproval", "initial request phase differs")
        admitted = stage("native-actual-admission-wire", lambda: actor.wait_event("admission-accepted"))
        keep_native_timing(admitted)
        admission_diagnostics = checked(sdk("path_diagnostics"))
        report["admissionWireObserved"] = purpose_wire_facts(
            admission_diagnostics.get("request", {}).get("alpn"),
            admission_diagnostics.get("gate", {}).get("coordinator"), "admission-request")
        if args.require_strict_purpose:
            stage("strict-restricted-admission-purpose", lambda: require_admission_wire(admission_diagnostics, admitted))
        native_admission = native_common("admission-request", admitted["stableId"])
        require(native_admission.get("purposeCode") == 2 and native_admission.get("restricted") is True
                and native_admission.get("permitted") == 0 and native_admission.get("peerPermitted") == 0
                and native_admission.get("ordinaryHandle") is False and native_admission.get("dataReleased") is False,
                "native admission control leaked data permission")
        report["nativeAdmissionPurpose"] = publish_sdk_facts(native_admission)
        pending = stage("exact-native-request",lambda:command("admission-status","admission-status"))
        requests = [value for value in pending["requests"] if value["applicant"] == browser_id]
        require(len(requests)==1, "exact authenticated browser request unavailable")
        collect_admission_events()
        probe_event_start = len(observed_admission_events)
        probe_generation = len(actor.runs)
        prior_probe_connections = {(entry["generation"],entry["event"]["stableId"])
            for entry in observed_admission_events if type(entry["event"].get("stableId")) is int}
        probe = stage("different-TLS-claim-probe",lambda:require_tls_probe_reply(sdk("admission_identity_probe",
            {"request":requests[0]["request"],"claimed":ready["id"]})))
        probe_end = stage("native-TLS-claim-handler-return",lambda:actor.wait_event(
            lambda event:event.get("event") == "admission-ended" and type(event.get("stableId")) is int
                and (probe_generation,event["stableId"]) not in prior_probe_connections))
        keep_native_timing(probe_end)
        collect_admission_events()
        after_probe = command("admission-status","admission-status")
        probe_facts = stage("strict-different-TLS-claim-rejection",lambda:require_tls_claim_rejection(
            probe,observed_admission_events[probe_event_start:],prior_probe_connections,probe_generation,pending,after_probe))
        for operation in ("open_member","sync_replica"):
            rejected = stage("unknown-applicant-"+operation+"-rejected",lambda operation=operation:
                require_unknown_data_rejection(sdk(operation,{"target":ready["id"]})))
        report["wrongIdentity"] = {**probe_facts,"businessRejected":True,"replicaRejected":True}
        approval = stage("exact-sdk-approval",lambda:command("approve-request","approved",applicant=browser_id,request=requests[0]["request"]))
        require(checked(sdk("request_admission",{"invitation":ready["invitation"]})) == "PendingMaterials", "authorization boundary unavailable")
        authorization = checked(sdk("material_receipt"))
        report["authorizationBoundary"] = material_facts(authorization)
        require(authorization["authorization_saved"] and not authorization["secret_saved"], "admission completed before interruption")
        stage("authorization-transaction-complete",lambda:fixture_facts(page,lambda value:value.get("authorizationOnlyCommitted") is True))
        stage("authorization-only-stop",stop_browser)
        reopened = stage("authorization-only-reopen",lambda:checked(sdk("start",{"configuration":configuration(ready)})))
        require(reopened["id"] == browser_id, "browser identity changed on interruption")
        require(checked(sdk("request_admission",{"invitation":ready["invitation"]})) == "PendingMaterials", "same admission did not resume")
        require(same_grant(authorization,checked(sdk("material_receipt"))), "request/grant changed after reopen")
        fault = stage("actual-secret-save-abort",lambda:sdk("complete_admission"))
        require(fault.get("ok") is False, "one-shot secret save failure was not observed")
        stage("actual-abort-event",lambda:fixture_facts(page,lambda value:value.get("secretAbortObserved") is True))
        stage("failed-save-stop",stop_browser)
        stage("failed-save-reopen",lambda:checked(sdk("start",{"configuration":configuration(ready)})))
        still_pending = checked(sdk("material_receipt"))
        require(still_pending["secret_saved"] is False and same_grant(authorization,still_pending), "aborted secret write was durable or changed grant")
        report["failedSecretSave"] = {"actualIdbAbort":True,"secretAbsentAfterReopen":True,"sameIdentityRequestGrant":True}
        require(checked(sdk("request_admission",{"invitation":ready["invitation"]})) == "PendingMaterials", "same-grant completion cannot resume")
        require(stage("same-grant-secret-save",lambda:checked(sdk("complete_admission"))) == "Complete", "final material completion unavailable")
        complete = checked(sdk("material_receipt"))
        report["completeMaterials"] = require_complete(complete)
        require(same_grant(authorization,complete), "completion changed saved grant")
        fixture = stage("secret-transaction-committed",lambda:fixture_facts(page,lambda value:any(tx.get("phase")=="secret" and tx.get("committed") is True for tx in value.get("transactions",[]))))
        report["actualTransactionFacts"] = fixture
        current = command("admission-status","admission-status")
        require(current["appliedCount"] == approval["after"], "interruption/resume duplicated approval/Add")
        stage("complete-browser-stop",stop_browser)
        author = stage("independent-offline-author",lambda:command("offline-author","offline-author"))
        require(author.get("independentIdentity") is True and author.get("providerVerifiedApplied") is True
                and author.get("bodyPresent") is False and author.get("serializedRecordBytes",0)>0,
                "actual SDK-authored nonempty record unavailable")
        cleanup = author["authorCleanup"]
        require(cleanup.get("ownedSocketsAbsent") is True and cleanup.get("ownedSocketsAbsentLate") is True, "author remains connected")
        target = author["record"]
        report["offlineAuthor"] = {name:author[name] for name in ("independentIdentity","source","serializedRecordBytes","bodyPresent","bodyBytes","providerVerifiedApplied","authorCleanup","savedProviderResourceConfigurationRestored")}
        current_phase = "provider-cleanup-before-reopen"
        native_retired = close_native(actor,remove_directory=False)
        keep_provider_cleanup(report,len(actor.runs),native_retired,"before-reopen")
        snapshot_execution("before-reopen")
        require(native_cleanup_check(native_retired,False), "saved provider did not retire")
        current_phase = "provider-restart"
        actor.start(native_args)
        reopened_native = stage("saved-provider-reopen",lambda:actor.wait_event("ready"))
        require(reopened_native["id"] == ready["id"], "provider identity changed on reopen")
        stage("complete-browser-reopen",lambda:checked(sdk("start",{"configuration":configuration(reopened_native,data=True)})))
        before = checked(sdk("material_receipt"))
        require_complete(before); require(same_grant(complete,before), "saved Complete identity/grant changed")
        replica = stage("actual-sdk-nonempty-replica",lambda:checked(sdk("sync_replica",{"target":reopened_native["id"]})))
        sender = stage("sender-durable-receipt",lambda:actor.wait_event("replica-finished"))
        keep_native_timing(sender)
        native_replica = native_common("replica", sender["stableId"])
        require(native_replica.get("purposeCode") == 3 and native_replica.get("freshAck") is True,
                "native replica lacked actual common fresh path ACK")
        report["nativeReplicaPurpose"] = publish_sdk_facts(native_replica)
        require(sender.get("durableReceiptAccepted") is True, "sender did not accept durable receipt")
        replica_diagnostics = checked(sdk("replica_diagnostics"))
        report["replicaWireObserved"] = purpose_wire_facts(
            replica_diagnostics.get("before", {}).get("alpn"),
            replica_diagnostics.get("before", {}).get("bootstrap", {}).get("coordinator"), "replica")
        if args.require_strict_purpose:
            stage("strict-current-member-replica-purpose", lambda: require_replica_wire(replica_diagnostics, sender))
        after = checked(sdk("material_receipt"))
        report["replica"] = require_replica(replica,target,before,after)
        stage("replica-browser-stop",stop_browser)
        stage("replica-browser-reopen",lambda:checked(sdk("start",{"configuration":configuration(reopened_native,data=True)})))
        persisted = checked(sdk("material_receipt"))
        require_complete(persisted); require(same_grant(complete,persisted) and target in persisted["records"], "replica target did not survive SDK reopen")
        report["replica"]["targetVerifiedSavedAfterReopen"] = True
        stage("ordinary-sdk-member",lambda:checked(sdk("open_member",{"target":reopened_native["id"]})))
        diagnostics = checked(sdk("member_diagnostics"))
        member_native = stage("native-current-business-member",lambda:actor.wait_event("member-released"))
        member_common = native_common("member-business", member_native["stableId"])
        require(member_native.get("remoteId") == browser_id and member_native.get("memberAuthorized") is True
                and member_native.get("alpn") == "miopunch/session/member/1"
                and member_common.get("purposeCode") == 1 and member_common.get("freshAck") is True,
                "native ordinary member lacks expected identity/current gate/path proof")
        report["nativeBusinessPurpose"] = publish_sdk_facts(member_common)
        report["nativeMember"] = publish_sdk_facts(member_native)
        require(diagnostics.get("newProtocol") is True and diagnostics.get("memberAuthorized") is True
                and diagnostics.get("expectedPeerMatched") is True and diagnostics.get("coordinator",{}).get("freshAck") is True
                and diagnostics.get("coordinator",{}).get("purposeCode") == 1,
                "current common ordinary-member gate/path proof missing")
        require(stage("ordinary-sdk-business",lambda:checked(sdk("exchange",{"message":"PING"}))) == "PONG", "SDK business response differs")
        business = stage("native-actual-business",lambda:actor.wait_event("business"))
        require(business.get("stableId") == member_native["stableId"] and business.get("bytes") == 4
                and business.get("memberMessage") is True and business.get("reply") == "PONG"
                and business.get("alpn") == "miopunch/session/member/1", "native business bytes or original M differ")
        report["ordinaryMember"] = publish_sdk_facts(diagnostics)
        report["business"] = {"browserToNativeBytes":4,"nativeToBrowserBytes":4,"replyMatched":True,
            "nativeOriginalMMatched":True,"native":publish_sdk_facts(business)}
        if args.relay:
            require(any(path.get("selected") is True and path.get("isRelay") is True
                        and path.get("route") == "relay:"+args.relay for path in diagnostics.get("paths",[]))
                    and member_common.get("selectedRoute") == "relay:"+args.relay
                    and any(path.get("selected") is True and path.get("isRelay") is True for path in member_native["paths"]),
                    "ordinary data did not select the explicit allowed relay")
        if args.rtc_disabled:
            disabled = checked(sdk("path_diagnostics"))["rtcCapability"]
            fixture = checked(sdk("rtc_fixture_snapshot"))
            retired = checked(sdk("rtc_diagnostics"))
            require(disabled.get("supported") is None and disabled.get("permitted") is False and disabled.get("effective") is False
                    and fixture.get("peerCreated") == 0 and fixture.get("channelCreated") == 0
                    and fixture.get("openPeerConnections") == 0 and fixture.get("openDataChannels") == 0,
                    "disabled RTC constructed a browser PC/DC or advertised capability")
            report["rtcDisabled"] = {"capability":disabled,"fixture":fixture,"sdk":retired,
                "nativeAdapterInstalled":False}
        stage("business-handle-close",lambda:checked(sdk("close_member")))
        removed = stage("actual-current-member-removal",lambda:command("remove-member","removed",target=browser_id))
        require(removed.get("currentAccessAbsent") is True,"SDK removal did not revoke access")
        for operation in ("open_member","sync_replica"):
            stage("revoked-"+operation+"-rejected",lambda operation=operation:
                require_unknown_data_rejection(sdk(operation,{"target":reopened_native["id"]})))
        old_grant_before = command("admission-status","admission-status")
        collect_admission_events()
        old_grant_event_start = len(observed_admission_events)
        old_grant_generation = len(actor.runs)
        prior_old_grant_connections = {(entry["generation"],entry["event"]["stableId"])
            for entry in observed_admission_events if type(entry["event"].get("stableId")) is int}
        old_grant = stage("invalid-old-grant-rejected",lambda:require_tls_probe_reply(
            sdk("request_admission",{"invitation":ready["invitation"]})))
        old_grant_end = stage("native-invalid-old-grant-handler-return",lambda:actor.wait_event(
            lambda event:event.get("event") == "admission-ended" and type(event.get("stableId")) is int
                and (old_grant_generation,event["stableId"]) not in prior_old_grant_connections))
        keep_native_timing(old_grant_end)
        collect_admission_events()
        old_grant_after = command("admission-status","admission-status")
        old_grant_facts = stage("strict-invalid-old-grant-rejection",lambda:require_old_grant_rejection(
            old_grant,observed_admission_events[old_grant_event_start:],prior_old_grant_connections,
            old_grant_generation,old_grant_before,old_grant_after))
        report["revokedAuthorization"] = {"actualSdkRemoval":True,"businessRejected":True,"replicaRejected":True,
            "oldGrantRejected":True,"freshRejoinApprovalRequested":False,"oldGrant":old_grant_facts}
        report["remainingQualification"] = []
        report["preparationPassed"] = True
    except Exception as error:
        remember_first_failure(report,error,current_phase)
        report["failureClass"] = type(error).__name__
        save()
    finally:
        report["cleanup"] = {}
        if observer is not None:
            try:
                value = observer.close()
            except Exception as error:
                value = {"forced":None,"commandExit":None,"readerEnded":None,
                    "failureClass":evidence_exception_class(error)}
            report["cleanup"]["browserObserver"] = value
            complete_observer = value.get("forced") is False and value.get("commandExit") == 0 and value.get("readerEnded") is True
            if not complete_observer:
                remember_first_failure(report,EvidenceCheckFailure("owned browser observer cleanup is incomplete"),"browser-observer-cleanup")
                report["preparationPassed"] = False
        if page:
            try:
                value = page.close(); report["cleanup"]["browser"] = value
                if not browser_cleanup_succeeded(value):
                    remember_first_failure(report,EvidenceCheckFailure("final browser cleanup is incomplete"),"final-browser-cleanup")
                report["preparationPassed"] &= browser_cleanup_succeeded(value)
            except Exception as error:
                remember_first_failure(report,error,"final-browser-cleanup")
                report["cleanup"]["browserFailureClass"] = type(error).__name__; report["preparationPassed"] = False
        if actor:
            try:
                value = close_native(actor); report["cleanup"]["native"] = value
                keep_provider_cleanup(report,len(actor.runs) or None,value,"final")
                if not native_cleanup_check(value):
                    remember_first_failure(report,EvidenceCheckFailure("final native cleanup is incomplete"),"final-native-cleanup")
                report["preparationPassed"] &= native_cleanup_check(value)
            except Exception as error:
                remember_first_failure(report,error,"final-native-cleanup")
                report["cleanup"]["nativeFailureClass"] = type(error).__name__; report["preparationPassed"] = False
        try:
            snapshot_execution("final")
        except Exception as error:
            remember_first_failure(report,error,"execution-evidence-retention")
            report["executionEvidenceFailureClass"] = type(error).__name__
            report["preparationPassed"] = False
        collect_admission_events()
        for operation,reply,ordinal in failed_admission_rpcs:
            retain_admission_observation(report,args.output,operation,reply,
                observed_admission_events,ordinal,"retired")
        try:
            report["artifactsUnchanged"] = sha256(binary)==deployed["deployedSha256"] and all(sha256(args.package/name)==digest for name,digest in wasm_build["artifacts"].items())
        except Exception as error:
            remember_first_failure(report,error,"final-artifact-guard")
            report["artifactsUnchanged"] = False
        if not report["artifactsUnchanged"]:
            remember_first_failure(report,EvidenceCheckFailure("SDK artifacts changed during the story"),"final-artifact-guard")
        report["preparationPassed"] &= report["artifactsUnchanged"]
        report["passed"] = report["preparationPassed"]
        report["newWireQualified"] = report["passed"]
        report["sdkPublicStoryQualified"] = report["passed"]
        # Existing SDK route/bytes are distinct from physical OS tuple/UID facts.
        report["physicalWanQualified"] = False
        report["physicalOsTuple"] = {"nativeSdkSocket":None,"browserPeerIpPort":None,"uidRouting":None}
        report["commandExit"] = 0 if report["passed"] else 1
        report["completedUtc"] = time.strftime("%Y-%m-%dT%H:%M:%SZ",time.gmtime())
        save()
    return report["commandExit"]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--development-checks",action="store_true")
    parser.add_argument("--execute",action="store_true")
    parser.add_argument("--allow-unqualified-preparation",action="store_true")
    parser.add_argument("--require-strict-purpose",action="store_true", help="require actual strict purpose, current gates and original-M path proof for full qualification")
    parser.add_argument("--relay",help="explicit permitted data relay; A still starts over protected MQTT")
    parser.add_argument("--rtc-disabled",action="store_true",help="omit Native RTC adapter and persist browser rtc_enabled=false")
    parser.add_argument("--native-platform",choices=("host","android"),default="host",
        help="Native actor platform; host avoids any Native Android device commands")
    parser.add_argument("--precode-receipt",type=Path,help="pinned affected short regression receipt")
    parser.add_argument("--admission-old-wire-observation",type=Path, help="bind the already captured rooted SDK admission RED without replay")
    for name in ("output","build-receipt","wasm-build-receipt","package","facility","native-profile","browser-profile"):
        parser.add_argument("--"+name,type=Path)
    args = parser.parse_args()
    if args.development_checks:
        require(not args.execute,"development checks and device execution are separate")
        print(json.dumps(development_checks(),indent=2)); return 0
    require(args.execute and args.allow_unqualified_preparation,"explicit SDK candidate execution selection is required")
    require(all(getattr(args,name.replace('-','_')) for name in ("output","build-receipt","wasm-build-receipt","package","facility","native-profile","browser-profile")),"all frozen artifacts and explicit resource profiles are required")
    require(args.precode_receipt,"affected short precode regression receipt is required")
    return run(args)


if __name__ == "__main__":
    try:
        code = main()
    except Exception as error:
        # A pre-operation guard must not create/overwrite an existing output.
        failed = {"commandExit":1}
        remember_first_failure(failed,error,"controller-preflight")
        print(json.dumps(failed,indent=2))
        code = 1
    raise SystemExit(code)
