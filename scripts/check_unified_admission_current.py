#!/usr/bin/env python3
"""Run the current M82 Android/Pixel admission story.

The M76 controller remains an immutable historical input.  This thin entry
point validates the current Android graph, installs the exact current-pair
runtime seam, and runs only the bounded M82 admission/reopen/live ordinary-M
story using its private DTO and cleanup primitives.  It never starts an actor
before the artifact guards finish.
"""

from __future__ import annotations

import argparse
import hashlib
import importlib
import json
import os
from pathlib import Path
import re
import subprocess
import time

ROOT = Path(__file__).resolve().parents[1]
CURRENT_SOURCE = ROOT / "src/bin/unified_admission_story.rs"
CURRENT_RUNTIME = ROOT / "scripts/android_pair_runtime_current.py"
CURRENT_BUILD_SCRIPT = ROOT / "scripts/build_android_pair.py"
NDK_REVISION = "29.0.14206865"
TARGET = "aarch64-linux-android"
ADMISSION_COMMIT_FIXTURE = ROOT / "browser-probe/harness/admission-commit-fixture.mjs"
RTC_CAPABILITY_FIXTURE = ROOT / "browser-probe/harness/rtc-capability-fixture.mjs"


class CurrentGuardError(RuntimeError):
    """A failed current-source/artifact precondition."""


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def require(condition: bool, message: str) -> None:
    if not condition:
        raise CurrentGuardError(message)


def _read_json(path: Path, label: str):
    require(path.is_file(), f"{label} is missing")
    try:
        return json.loads(path.read_text())
    except (OSError, ValueError) as error:
        raise CurrentGuardError(f"{label} is not valid JSON") from error


def _command_has(command: list[str], *values: str) -> bool:
    return all(value in command for value in values)


def _m82_prelude() -> str:
    """Build the current page prelude with M83's abort fault disabled."""

    require(ADMISSION_COMMIT_FIXTURE.is_file() and RTC_CAPABILITY_FIXTURE.is_file(),
            "current browser fixture sources are missing")
    script = (
        "import {admissionCommitPrelude} from " + json.dumps(ADMISSION_COMMIT_FIXTURE.as_uri())
        + ";import {rtcCapabilityPrelude} from " + json.dumps(RTC_CAPABILITY_FIXTURE.as_uri())
        + ";process.stdout.write(rtcCapabilityPrelude('observe')+'\\n'+"
        + "admissionCommitPrelude({abortSecretOnce:false}));"
    )
    executed = subprocess.run(["rtk", "proxy", "node", "--input-type=module", "-e", script],
                              check=True, capture_output=True, text=True)
    # This wrapper observes only method boundaries and coarse WebRTC states.
    # It never reads SDP/ICE values, installs event callbacks, creates a
    # connection/channel, retries an operation, or changes a default.
    return executed.stdout + r'''
(function installM82RtcProgress() {
  const actor = new URL(location.href).searchParams.get('actor');
  const started = performance.now();
  const progress = { instrumented: false, sequence: 0, peerCount: 0,
    dataChannelCount: 0, closeCount: 0, events: [] };
  const peerIds = new WeakMap();
  const statsSeen = new WeakSet();
  let nextPeerId = 1;
  const peerId = peer => {
    if (!peer || (typeof peer !== 'object' && typeof peer !== 'function')) return null;
    if (!peerIds.has(peer)) peerIds.set(peer, nextPeerId++);
    return peerIds.get(peer);
  };
  const readState = (peer, name) => {
    try { return peer?.[name] ?? null; } catch (_) { return null; }
  };
  const coarseState = peer => ({
    connectionState: readState(peer, 'connectionState'),
    iceConnectionState: readState(peer, 'iceConnectionState'),
    iceGatheringState: readState(peer, 'iceGatheringState'),
  });
  const publish = () => {
    try {
      const request = fetch(`/result?actor=${encodeURIComponent(actor)}`, {
        method: 'POST', headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ id: 'mio82-rtc-progress', ok: true, result: {
          scope: 'owned read-only RTC method/state progress', instrumented: progress.instrumented,
          sequence: progress.sequence, peerCount: progress.peerCount,
          dataChannelCount: progress.dataChannelCount, closeCount: progress.closeCount,
          events: progress.events.slice(-256),
        } }),
      });
      if (request && typeof request.catch === 'function') request.catch(() => {});
    } catch (_) {}
  };
  const record = (kind, peer, extra = {}) => {
    try {
      progress.sequence++;
      if (progress.events.length >= 256) progress.events.shift();
      progress.events.push({ kind, peerId: peerId(peer), sequence: progress.sequence,
        atMs: performance.now() - started, state: coarseState(peer), ...extra });
      publish();
    } catch (_) {}
  };
  const instrumentChannel = (peer, channel) => {
    try {
      if (!channel || channel.__mio82ProgressWrapped) return;
      const close = channel.close;
      if (typeof close === 'function') {
        try {
          Object.defineProperty(channel, 'close', { configurable: true, writable: true,
            value(...args) {
              record('datachannel-close-start', peer, { dataChannelState: readState(channel, 'readyState') });
              try {
                const result = close.apply(this, args);
                record('datachannel-close-end', peer, { dataChannelState: readState(channel, 'readyState'), ok: true });
                return result;
              } catch (error) {
                record('datachannel-close-end', peer, { dataChannelState: readState(channel, 'readyState'),
                  ok: false, failureClass: error?.name ?? typeof error });
                throw error;
              }
            } });
        } catch (error) {
          record('observer-install-failure', peer, { target: 'datachannel.close',
            failureClass: error?.name ?? typeof error });
        }
      }
      try { Object.defineProperty(channel, '__mio82ProgressWrapped', { value: true }); } catch (_) {}
    } catch (error) {
      record('observer-install-failure', peer, { target: 'datachannel',
        failureClass: error?.name ?? typeof error });
    }
  };
  // Read-only summaries on this page's actual PC; host UDP port hints stay
  // in the private transcript for capture filters. No SDP/IP/key is recorded.
  const actualPeers = new WeakSet();
  const acceptedSeen = new WeakSet();
  const preCloseSeen = new WeakSet();
  const summarizeSdp = description => {
    const counts = { host: 0, srflx: 0, prflx: 0, relay: 0, other: 0,
      udp: 0, tcp: 0, otherProtocol: 0, relatedAddressPresent: 0 };
    const sdp = description?.sdp;
    const result = { present: typeof sdp === 'string', oversized: false,
      descriptionType: ['offer', 'answer'].includes(description?.type) ? description.type : null,
      setupActive: false, setupPassive: false, setupActpass: false,
      iceLite: false, endOfCandidates: false, hostUdpPorts: [], counts };
    if (typeof sdp !== 'string') return result;
    if (sdp.length > 65536) { result.oversized = true; return result; }
    for (const line of sdp.split(/\r?\n/)) {
      if (line === 'a=setup:active') result.setupActive = true;
      if (line === 'a=setup:passive') result.setupPassive = true;
      if (line === 'a=setup:actpass') result.setupActpass = true;
      if (line === 'a=ice-lite') result.iceLite = true;
      if (line === 'a=end-of-candidates') result.endOfCandidates = true;
      if (!line.startsWith('a=candidate:')) continue;
      const fields = line.split(/\s+/);
      const type = fields[7];
      counts[['host', 'srflx', 'prflx', 'relay'].includes(type) ? type : 'other']++;
      const protocol = fields[2]?.toLowerCase();
      counts[['udp', 'tcp'].includes(protocol) ? protocol : 'otherProtocol']++;
      const port = Number(fields[5]);
      if (type === 'host' && protocol === 'udp' && Number.isInteger(port) && port > 0 && port <= 65535 &&
          !result.hostUdpPorts.includes(port) && result.hostUdpPorts.length < 16) result.hostUdpPorts.push(port);
      if (fields.includes('raddr')) counts.relatedAddressPresent++;
    }
    return result;
  };
  const summarizeStats = report => {
    const result = { local: {host:0,srflx:0,prflx:0,relay:0,other:0},
      remote: {host:0,srflx:0,prflx:0,relay:0,other:0},
      pairCount: 0, nominatedCount: 0, selectedPairReferenced: false,
      pairStates: {frozen:0,waiting:0,'in-progress':0,failed:0,succeeded:0,other:0},
      requestsSent: 0, requestsReceived: 0, responsesSent: 0, responsesReceived: 0,
      bytesSent: 0, bytesReceived: 0,
      dtlsStates: {new:0,connecting:0,connected:0,closed:0,failed:0,other:0} };
    report.forEach(entry => {
      if (entry.type === 'local-candidate' || entry.type === 'remote-candidate') {
        const side = entry.type === 'local-candidate' ? result.local : result.remote;
        side[['host','srflx','prflx','relay'].includes(entry.candidateType) ? entry.candidateType : 'other']++;
      } else if (entry.type === 'candidate-pair') {
        result.pairCount++;
        result.pairStates[Object.hasOwn(result.pairStates, entry.state) ? entry.state : 'other']++;
        if (entry.nominated === true) result.nominatedCount++;
        for (const key of ['requestsSent','requestsReceived','responsesSent','responsesReceived','bytesSent','bytesReceived']) {
          if (Number.isFinite(entry[key]) && entry[key] >= 0) result[key] += entry[key];
        }
      } else if (entry.type === 'transport') {
        if (typeof entry.selectedCandidatePairId === 'string' && entry.selectedCandidatePairId.length > 0)
          result.selectedPairReferenced = true;
        result.dtlsStates[Object.hasOwn(result.dtlsStates,entry.dtlsState) ? entry.dtlsState : 'other']++;
      }
    });
    return result;
  };
  const observeActualPeer = (peer, phase, originalStats) => {
    try {
    if (!actualPeers.has(peer)) return;
    record('m82-candidate-summary', peer, { phase,
      local: summarizeSdp(peer.localDescription), remote: summarizeSdp(peer.remoteDescription) });
    if (typeof originalStats !== 'function') {
      record('m82-stats-summary', peer, { phase, available: false }); return;
    }
    try {
      // Start before close, never await or extend the SDK caller/cleanup deadline.
      Promise.resolve(originalStats.call(peer)).then(report => {
        record('m82-stats-summary', peer, { phase, available: true, summary: summarizeStats(report) });
      }, () => record('m82-stats-summary', peer, { phase, available: false }))
        .catch(() => record('m82-stats-summary', peer, { phase, available: false }));
    } catch (_) { record('m82-stats-summary', peer, { phase, available: false }); }
    } catch (_) { /* Observer errors must never affect the SDK method. */ }
  };
  const instrumentPeer = peer => {
    let originalStats;
    try { originalStats = peer.getStats; }
    catch (_) {
      record('m82-stats-summary', peer, { phase: 'observer-install', available: false });
    }
    for (const name of ['createOffer', 'createAnswer', 'setLocalDescription',
      'setRemoteDescription', 'addIceCandidate', 'getStats', 'close', 'createDataChannel']) {
      let original;
      try { original = peer[name]; }
      catch (error) {
        record('observer-install-failure', peer, { target: name,
          failureClass: error?.name ?? typeof error });
        continue;
      }
      if (typeof original !== 'function' || original.__mio82ProgressWrapped) continue;
      const wrapped = function(...args) {
        if (name === 'getStats') {
          if (statsSeen.has(this)) return original.apply(this, args);
          statsSeen.add(this);
        }
        if (name === 'setRemoteDescription') actualPeers.add(this);
        if (name === 'close' && actualPeers.has(this) && !preCloseSeen.has(this)) {
          preCloseSeen.add(this);
          observeActualPeer(this, 'before-pc-close', originalStats);
        }
        if (name === 'close') progress.closeCount++;
        const call = progress.sequence + 1;
        record('method-start', this, { method: name, callId: call });
        let result;
        try { result = original.apply(this, args); }
        catch (error) {
          record('method-end', this, { method: name, callId: call, ok: false,
            failureClass: error?.name ?? typeof error });
          throw error;
        }
        const finish = value => {
          if (name === 'setRemoteDescription' && !acceptedSeen.has(this)) {
            acceptedSeen.add(this);
            observeActualPeer(this, 'remote-description-accepted', originalStats);
          }
          if (name === 'createDataChannel' && value) {
            progress.dataChannelCount++;
            instrumentChannel(this, value);
            record('datachannel-created', this, { callId: call,
              dataChannelState: readState(value, 'readyState') });
          }
          record('method-end', this, { method: name, callId: call, ok: true });
          return value;
        };
        if (result && typeof result.then === 'function') {
          try {
            result.then(value => { finish(value); }, error => {
              record('method-end', this, { method: name, callId: call, ok: false,
                failureClass: error?.name ?? typeof error });
            }).catch(() => {});
          } catch (_) {}
          return result;
        }
        return finish(result);
      };
      try {
        Object.defineProperty(wrapped, '__mio82ProgressWrapped', { value: true });
        Object.defineProperty(peer, name, { configurable: true, writable: true, value: wrapped });
      } catch (error) {
        record('observer-install-failure', peer, { target: name,
          failureClass: error?.name ?? typeof error });
      }
    }
    record('peer-created', peer, {});
  };
  const ExistingPeer = globalThis.RTCPeerConnection;
  try {
    if (typeof ExistingPeer !== 'function') throw new Error('RTCPeerConnection unavailable');
    function ObservedPeer(...args) {
      const peer = new ExistingPeer(...args);
      progress.peerCount++;
      instrumentPeer(peer);
      return peer;
    }
    ObservedPeer.prototype = ExistingPeer.prototype;
    Object.setPrototypeOf(ObservedPeer, ExistingPeer);
    Object.defineProperty(globalThis, 'RTCPeerConnection', {
      configurable: true, writable: true, value: ObservedPeer,
    });
    progress.instrumented = true;
  } catch (error) {
    progress.instrumentFailureClass = error?.name ?? typeof error;
  }
  globalThis.__mio82RtcProgress = progress;
  publish();
})();
'''


def _source_entry(sources: dict, suffix: str):
    """Resolve one repository-root input without accepting path aliases.

    The Android source inventory also contains vendor manifests.  Suffix
    matching therefore cannot identify the repository's Cargo.toml.  Keep the
    inventory's canonical repository-relative key, reject an absolute or
    external alias for the same file, and leave digest verification to the
    caller.
    """
    expected_key = Path(suffix).as_posix()
    require(not expected_key.startswith("../") and not Path(expected_key).is_absolute(),
            f"source entry suffix escapes the repository: {suffix}")
    canonical = (ROOT / expected_key).resolve()
    require(canonical.is_file(), f"current source entry is missing: {expected_key}")
    exact = []
    aliases = []
    for raw_name, digest in sources.items():
        name = str(raw_name).replace("\\", "/")
        if name == expected_key:
            exact.append((raw_name, digest))
            continue
        if name.startswith("external:"):
            continue
        try:
            if Path(raw_name).expanduser().resolve() == canonical:
                aliases.append(raw_name)
        except (OSError, RuntimeError, ValueError):
            continue
    require(not aliases, f"source snapshot contains an alias for {expected_key}")
    require(len(exact) == 1, f"current source snapshot must contain exactly one {expected_key}")
    return exact[0]


def _source_snapshot(receipt_path: Path, name: str) -> dict:
    snapshot = receipt_path.parent / name
    return _read_json(snapshot, name)


def _elf_arm64(path: Path) -> bool:
    data = path.read_bytes()[:20]
    if len(data) < 20 or data[:4] != b"\x7fELF" or data[4] != 2:
        return False
    endian = "little" if data[5] == 1 else "big" if data[5] == 2 else None
    return endian is not None and int.from_bytes(data[18:20], endian) == 183


def _validate_actual_selected_inputs(receipt_path: Path, receipt: dict) -> dict:
    selected_path = receipt_path.parent / "selected-dependency-inputs.json"
    manifest_path = Path(receipt.get("actualAndroidManifest", ""))
    require(selected_path.is_file() and manifest_path.is_file(),
            "actual Android selected-input/manifest receipts are mandatory")
    selected = _read_json(selected_path, "actual Android selected dependency inputs")
    manifest = _read_json(manifest_path, "actual Android manifest")
    require(manifest_path.is_relative_to(receipt_path.parent),
            "actual Android manifest is outside the immutable receipt")
    inputs = selected.get("inputs")
    copies = selected.get("copiedRepositoryInputs")
    depfiles = selected.get("capturedDepfiles")
    logs = selected.get("rawLogs")
    require(isinstance(inputs, dict) and inputs and isinstance(copies, dict)
            and set(copies) == set(inputs) and isinstance(depfiles, dict) and depfiles
            and isinstance(logs, dict), "actual Android selected input sets are incomplete")
    require(selected.get("manifest", {}).get("sha256") == sha256(manifest_path)
            and receipt.get("actualAndroidManifestSha256") == sha256(manifest_path),
            "actual Android manifest hash binding differs")
    for raw, expected in inputs.items():
        source = Path(raw)
        require(source.is_file() and sha256(source) == expected,
                "actual selected Android compiler input changed: " + raw)
        copy = copies[raw]
        copy_path = Path(copy.get("path", "")) if isinstance(copy, dict) else Path("")
        require(copy_path.is_relative_to(receipt_path.parent / "selected-source-copies")
                and copy_path.is_file()
                and sha256(Path(copy["path"])) == copy.get("sha256"),
                "captured Android source copy changed: " + raw)
    current_source_inputs = [value for key, value in inputs.items()
                             if str(key).replace("\\", "/").endswith("src/bin/unified_admission_story.rs")]
    require(current_source_inputs == [sha256(CURRENT_SOURCE)],
            "actual Android depfile set does not bind the current CLI source")
    for raw, expected in depfiles.items():
        path = Path(raw)
        require(path.is_file() and sha256(path) == expected,
                "captured Android depfile changed: " + raw)
        require(path.parent == receipt_path.parent / "captured-depfiles",
                "captured Android depfile is outside the immutable receipt")
    for name, expected in logs.items():
        path = receipt_path.parent / name
        require(path.is_file() and sha256(path) == expected,
                "captured Android raw log changed: " + name)
    require(manifest.get("platform") == "android" and manifest.get("target") == TARGET
            and manifest.get("binary") == "unified_admission_story"
            and manifest.get("locked") is True and manifest.get("offline") is True
            and manifest.get("jniShell") is False and manifest.get("features") == ["rtc-probe"],
            "actual Android manifest does not prove the locked CLI graph")
    require(manifest.get("toolProof") and manifest.get("compilerProof")
            and manifest.get("ndkRevision") == NDK_REVISION,
            "actual Android manifest lacks tool/NDK proof")
    require(manifest.get("selectedInputCount") == len(inputs)
            and manifest.get("sourceCopyCount") == len(copies)
            and manifest.get("capturedDepfileCount") == len(depfiles),
            "actual Android manifest counts do not match selected sets")
    require(receipt.get("actualSelectedInputCount") == len(inputs)
            and receipt.get("actualSourceCopyCount") == len(copies)
            and receipt.get("actualCapturedDepfileCount") == len(depfiles),
            "Android receipt counts do not derive from the actual manifest")
    return {"manifestSha256": sha256(manifest_path), "selectedInputsSha256": sha256(selected_path),
            "selectedInputCount": len(inputs), "sourceCopyCount": len(copies),
            "capturedDepfileCount": len(depfiles), "rawLogCount": len(logs)}


def guard_current_android_build(receipt_path: Path, expected_sha256: str | None = None) -> dict:
    """Validate the one current Android CLI graph without invoking Cargo."""

    require(receipt_path.is_file(), "current Android build receipt is missing")
    if expected_sha256 is not None:
        require(re.fullmatch(r"[0-9a-f]{64}", expected_sha256) is not None,
                "native build receipt SHA-256 must be lowercase hexadecimal")
        require(sha256(receipt_path) == expected_sha256,
                "current Android build receipt hash differs from the selected pin")
    receipt = _read_json(receipt_path, "current Android build receipt")
    require(receipt.get("exit") == 0 and receipt.get("cargoExit") == 0,
            "current Android Cargo graph did not finish successfully")
    require(receipt.get("qualifiedForFrozenRun") is True
            and receipt.get("sourceFrozenDuringBuild") is True,
            "current Android receipt is not source-frozen")
    command = [str(value) for value in receipt.get("command", [])]
    require(_command_has(command, "--locked", "--offline", "--target", TARGET,
                         "--features", "rtc-probe", "--bin", "unified_admission_story"),
            "current Android command must be the locked offline rtc-probe CLI graph")
    require("--jni-shell" not in command and "--d8-jar" not in command,
            "current Android graph must not include JNI or shell.jar")
    bins = [command[index + 1] for index, value in enumerate(command[:-1])
            if value == "--bin"]
    require(bins == ["unified_admission_story"],
            "current Android graph must contain exactly one selected CLI binary")
    require(Path(str(receipt.get("targetDirectory", ""))).resolve() == (ROOT / "target").resolve(),
            "current Android build must use the shared repository target")
    require(re.search(r"Pkg\.Revision\s*=\s*" + re.escape(NDK_REVISION) + r"(?:\s|$)",
                      str(receipt.get("ndk", ""))) is not None,
            "current Android build must use pinned NDK 29.0.14206865")

    before = _source_snapshot(receipt_path, "sources-before.json")
    after = _source_snapshot(receipt_path, "sources-after.json")
    require(before == after, "current Android source snapshot changed during build")
    source_name, source_digest = _source_entry(before, "src/bin/unified_admission_story.rs")
    require(source_digest == sha256(CURRENT_SOURCE),
            "Android receipt does not select the current unified_admission_story source")
    for suffix in ("Cargo.toml", "Cargo.lock", ".cargo/config.toml"):
        name, digest = _source_entry(before, suffix)
        current = ROOT / name if not str(name).startswith("external:") else None
        require(current is not None and current.is_file() and sha256(current) == digest,
                f"current Android {suffix} source/config changed after the build")

    artifacts = receipt.get("deployment", {}).get("unified_admission_story")
    require(isinstance(artifacts, dict), "current Android deployment pin is missing")
    for key in ("compiledPath", "deployedPath", "compiledSha256", "deployedSha256"):
        require(key in artifacts, f"current Android deployment field {key} is missing")
    compiled = Path(artifacts["compiledPath"])
    deployed = Path(artifacts["deployedPath"])
    require(compiled.is_file() and deployed.is_file(),
            "current Android compiled/deployed ELF is missing")
    require(compiled.resolve().is_relative_to((ROOT / "target" / TARGET).resolve()),
            "current Android compiled artifact is outside the shared target triple")
    require(sha256(compiled) == artifacts["compiledSha256"]
            and sha256(deployed) == artifacts["deployedSha256"],
            "current Android compiled/deployed ELF hash differs from its receipt")
    require(_elf_arm64(compiled) and _elf_arm64(deployed),
            "current Android compiled/deployed artifacts are not ARM64 ELF")
    deployment = receipt["deployment"]["unified_admission_story"]
    pinned = Path(deployment.get("pinnedCompiledPath", ""))
    require(pinned.is_relative_to(receipt_path.parent / "pinned-compiled")
            and pinned.is_file() and sha256(pinned) == deployment.get("pinnedCompiledSha256")
            and _elf_arm64(pinned), "current Android pinned compiled ELF is missing or changed")
    require(Path(deployment.get("deployedPath", "")).is_relative_to(receipt_path.parent),
            "current Android deployed artifact is outside the immutable receipt")
    require(deployment.get("transformation", {}).get("exit") == 0,
            "current Android compile-to-pin-to-deploy transformation failed")
    actual_inputs = _validate_actual_selected_inputs(receipt_path, receipt)

    return {
        "receiptSha256": sha256(receipt_path),
        "sourceSnapshotSha256": sha256(receipt_path.parent / "sources-before.json"),
        "currentSourceSha256": sha256(CURRENT_SOURCE),
        "depfile": next(iter(json.loads((receipt_path.parent / "selected-dependency-inputs.json").read_text())["capturedDepfiles"])),
        "depfileSha256": next(iter(json.loads((receipt_path.parent / "selected-dependency-inputs.json").read_text())["capturedDepfiles"].values())),
        "selectedDependencyInputsSha256": actual_inputs["selectedInputsSha256"],
        "actualManifestSha256": actual_inputs["manifestSha256"],
        "actualSelectedInputCount": actual_inputs["selectedInputCount"],
        "actualSourceCopyCount": actual_inputs["sourceCopyCount"],
        "actualCapturedDepfileCount": actual_inputs["capturedDepfileCount"],
        "compiledSha256": artifacts["compiledSha256"],
        "deployedSha256": artifacts["deployedSha256"],
        "compiledPath": str(compiled),
        "deployedPath": str(deployed),
        "target": TARGET,
        "feature": "rtc-probe",
        "ndkRevision": NDK_REVISION,
        "binary": "unified_admission_story",
    }


def _guard_frozen_browser_build(receipt_path: Path, package: Path) -> dict:
    """Cheap pre-actor guard; the frozen controller performs the full bind."""

    receipt = _read_json(receipt_path, "current browser build receipt")
    require(receipt.get("qualifiedForFrozenRun") is True,
            "current browser package receipt is not source-frozen")
    artifacts = receipt.get("artifacts")
    require(isinstance(artifacts, dict) and artifacts,
            "current browser package artifact list is missing")
    for name, digest in artifacts.items():
        path = package / name
        require(path.is_file() and sha256(path) == digest,
                f"current browser package artifact changed: {name}")
    return {"receiptSha256": sha256(receipt_path), "packageSha256": _directory_sha256(package),
            "artifactCount": len(artifacts)}


def _directory_sha256(directory: Path) -> str:
    entries = []
    for path in sorted(directory.rglob("*")):
        if path.is_file():
            entries.append((str(path.relative_to(directory)), sha256(path)))
    return hashlib.sha256(json.dumps(entries, sort_keys=True).encode()).hexdigest()


def _private_write(path: Path, value: object) -> None:
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(descriptor, "w") as stream:
        json.dump(value, stream, indent=2)
        stream.write("\n")
    path.chmod(0o600)


def _selected_pair(value: object) -> dict | None:
    """Find the current selected candidate pair without using a stale route."""

    if not isinstance(value, dict):
        return None
    def find(candidate: object, depth: int = 0) -> dict | None:
        if depth > 10 or not isinstance(candidate, dict):
            return None
        direct = candidate.get("selectedPair")
        if isinstance(direct, dict) and isinstance(direct.get("local"), dict) \
                and isinstance(direct.get("remote"), dict):
            return direct
        if isinstance(direct, dict):
            local = candidate.get("localCandidate")
            remote = candidate.get("remoteCandidate")
            transport = candidate.get("transport")
            if (isinstance(local, dict) and isinstance(remote, dict)
                    and isinstance(transport, dict)
                    and direct.get("state") == "succeeded" and direct.get("nominated") is True
                    and isinstance(direct.get("id"), str)
                    and transport.get("selectedCandidatePairId") == direct["id"]
                    and isinstance(local.get("id"), str) and isinstance(remote.get("id"), str)
                    and direct.get("localCandidateId") == local["id"]
                    and direct.get("remoteCandidateId") == remote["id"]):
                return {"local": local, "remote": remote}
        for child in candidate.values():
            found = find(child, depth + 1)
            if found is not None:
                return found
        return None

    pair = find(value)
    if pair is None:
        return None

    def endpoint(raw, require_base: bool):
        if not isinstance(raw, dict):
            return None
        address = raw.get("address") or raw.get("ip")
        port = raw.get("port")
        candidate_type = raw.get("candidateType", raw.get("typ", raw.get("type")))
        protocol = raw.get("protocol")
        if not address and not require_base and candidate_type == "prflx":
            # Chrome may suppress this address. A live packet witness must
            # measure it before the route collector receives a usable tuple.
            address = ""
        if (not isinstance(address, str) or type(port) is not int or not 1 <= port <= 65535
                or candidate_type not in {"host", "srflx", "prflx", "relay"}
                or (protocol is not None and str(protocol).lower() != "udp")):
            return None
        result = {"address": address, "port": port, "candidateType": candidate_type,
                  "protocol": str(protocol).lower() if protocol is not None else None,
                  "relatedAddress": raw.get("relatedAddress", raw.get("related_address")),
                  "relatedPort": raw.get("relatedPort", raw.get("related_port"))}
        if result["relatedAddress"] in (None, "") and result["relatedPort"] in (None, 0):
            result["relatedAddress"], result["relatedPort"] = None, None
        if candidate_type in {"srflx", "prflx"}:
            if not isinstance(result["relatedAddress"], str) or type(result["relatedPort"]) is not int:
                if require_base:
                    return None
                result["baseAddress"], result["basePort"] = address, port
            else:
                result["baseAddress"], result["basePort"] = result["relatedAddress"], result["relatedPort"]
        else:
            result["baseAddress"], result["basePort"] = address, port
        return result

    local, remote = endpoint(pair.get("local"), True), endpoint(pair.get("remote"), False)
    if local is None or remote is None:
        return None
    return {"local": local, "remote": remote, "nonRelay":
            local["candidateType"] != "relay" and remote["candidateType"] != "relay"}


def _browser_normal_tls(diagnostics: object) -> bool:
    """Require a live browser WSS MQTT CONNACK/SUBACK report."""

    if not isinstance(diagnostics, dict):
        return False
    carriers = diagnostics.get("carriers")
    if not isinstance(carriers, list):
        return False
    for carrier in carriers:
        if not isinstance(carrier, dict) or carrier.get("protocol") != "mqtt":
            continue
        report = carrier.get("traffic", carrier)
        if not isinstance(report, dict):
            continue
        if report.get("connack") == 0 and report.get("suback_codes") == [0] \
                and report.get("ready_ms") is not None:
            return True
    return False


def _observer_state(value: object, stable: int) -> bool:
    return isinstance(value, dict) and value.get("stableId") == stable \
        and value.get("sameOriginal") is True and value.get("alive") is True \
        and value.get("memberGate") is True and value.get("selectedRtc") is True


def _live_member_local_ids(native_event: dict, browser_diagnostics: dict,
                           browser_id: str) -> tuple[int, int]:
    """Validate peer gates and each endpoint's own connection identifier."""

    native_id = native_event.get("stableId")
    require(type(native_id) is int and native_event.get("memberAuthorized") is True
            and native_event.get("newProtocol") is True
            and native_event.get("remoteId") == browser_id,
            "current Native member gate was not observed")
    require(browser_diagnostics.get("memberAuthorized") is True
            and browser_diagnostics.get("newProtocol") is True
            and browser_diagnostics.get("expectedPeerMatched") is True
            and browser_diagnostics.get("ordinaryHandle") is True,
            "current Browser member gate was not observed")
    bootstrap = browser_diagnostics.get("bootstrap", {})
    coordinator = browser_diagnostics.get("coordinator", {})
    require(isinstance(bootstrap, dict), "browser member bootstrap is unavailable")
    if not isinstance(coordinator, dict) or not coordinator:
        coordinator = bootstrap.get("coordinator", {})
    browser_local_id = bootstrap.get("memberStableId")
    require(type(browser_local_id) is int and isinstance(coordinator, dict)
            and coordinator.get("stableId") == browser_local_id,
            "browser diagnostics changed their local original M")
    # These IDs belong to different processes; peer identity checks above
    # associate the endpoints, while continuity compares IDs only locally.
    return native_id, browser_local_id


def _native_live_observation(mark_reply: dict | None, stable: int):
    """Read only the M77-owned live association DTO; reject caller tuples."""

    if not isinstance(mark_reply, dict) \
            or mark_reply.get("currentReplyABI") != "member-socket-mark/v3":
        return None, None, None, "native-live-observer-unavailable"
    if not _observer_state(mark_reply.get("originalM"), stable) \
            or not _observer_state(mark_reply.get("after"), stable):
        return None, None, None, "native-live-observer-state-mismatch"
    native_pair = _selected_pair({"selectedPair": mark_reply.get("selectedPair")})
    if native_pair is None:
        return None, None, None, "native-selected-pair-unavailable"
    socket_tuple = mark_reply.get("socketTuple")
    if not isinstance(socket_tuple, dict):
        return None, None, None, "rtc-association-local-tuple-unavailable"
    expected = {
        "localAddress": native_pair["local"]["baseAddress"],
        "localPort": native_pair["local"]["basePort"],
        "peerAddress": native_pair["remote"]["address"],
        "peerPort": native_pair["remote"]["port"],
    }
    if socket_tuple != expected:
        return None, None, None, "native-selected-pair-tuple-mismatch"
    observation = mark_reply.get("observation", mark_reply.get("tupleCorrelation"))
    if not isinstance(observation, dict):
        return None, None, None, "native-socket-observation-unavailable"
    if observation.get("duplicatesClosed") is not True:
        return None, None, None, "native-socket-duplicate-cleanup-unverified"
    return native_pair, expected, observation, None


def _route_request_from_live_m(native_event: dict, member_diagnostics: dict,
                               mark_reply: dict | None, native_directory: str,
                               packet_witness: dict | None = None) -> tuple[dict | None, str | None]:
    """Build a route request from the M77-owned live observer and Browser DTO."""

    browser_pair = _selected_pair(member_diagnostics)
    stable = native_event.get("stableId")
    if type(stable) is not int:
        return None, "native-live-m-observer-unavailable"
    native_pair, native_tuple, observation, observer_gap = _native_live_observation(mark_reply, stable)
    if browser_pair is None:
        return None, "selected-pc-stats-unavailable"
    if observer_gap is not None:
        return None, observer_gap
    if not browser_pair["remote"]["address"]:
        if packet_witness is None:
            return None, "selected-pc-stats-unavailable"
        from android_pair_packet_witness import derive_missing_peer_address
        measured = derive_missing_peer_address(
            packet_witness, native_tuple, browser_pair["local"]["baseAddress"],
            browser_pair["local"]["basePort"], browser_pair["remote"]["port"])
        if measured is None:
            return None, "selected-pair-mismatch"
        browser_pair["remote"].update(address=measured, baseAddress=measured,
            addressObservation="matched Native ICE payload in Browser capture")
    if not browser_pair.get("nonRelay") or not native_pair.get("nonRelay"):
        return None, "selected-pair-relay"
    if packet_witness is None and ((native_pair["local"]["address"], native_pair["local"]["port"]) != \
            (browser_pair["remote"]["address"], browser_pair["remote"]["port"]) \
            or (native_pair["remote"]["address"], native_pair["remote"]["port"]) != \
            (browser_pair["local"]["address"], browser_pair["local"]["port"])):
        return None, "selected-pair-mismatch"
    native_observation = None
    if observation.get("observed") is True:
        candidate = observation.get("socketEvidence")
        if isinstance(candidate, dict):
            native_observation = candidate
    # Native srflx/prflx local addresses are mapped wire candidates.  The
    # related host candidate is the only address eligible for an OS route
    # lookup; the remote side remains the selected wire candidate.
    browser_tuple = {
        "localAddress": browser_pair["local"]["baseAddress"], "localPort": browser_pair["local"]["basePort"],
        "peerAddress": browser_pair["remote"]["address"], "peerPort": browser_pair["remote"]["port"]}
    member = {"sameOriginal": True, "alive": True, "memberGate": True,
              "selectedRtc": True, "localTuple": native_tuple}
    native_endpoint = {"role": "native", "model": "PJZ110", "transport": "usb",
         "actor": {"kind": "native", "ownedDirectory": native_directory},
         "selectedTuple": native_tuple, "currentM": member,
         "selectedPair": native_pair}
    if native_observation is not None:
        native_endpoint["socketEvidence"] = {"observed": True, "socketEvidence": native_observation}
    request = {"endpoints": [
        native_endpoint,
        {"role": "browser", "model": "Pixel_6a", "transport": "usb",
         "actor": {"kind": "chrome", "package": "com.android.chrome"},
         "selectedTuple": browser_tuple, "currentM": {
             "sameOriginal": True, "alive": True, "memberGate": True,
             "selectedRtc": True, "localTuple": browser_tuple},
         "selectedPair": browser_pair,
         "pcStats": {"source": "browser-owned-selected-pc", "selected": True,
                     "sameOriginal": True, "alive": True, "memberGate": True,
                     "localTuple": browser_tuple,
                     "peerTuple": {"localAddress": native_tuple["localAddress"],
                                   "localPort": native_tuple["localPort"],
                                   "peerAddress": browser_tuple["peerAddress"],
                                   "peerPort": browser_tuple["peerPort"]},
                     "pingCount": 1, "pongCount": 1}},
    ]}
    if packet_witness is not None:
        request["packetWitness"] = packet_witness
    return request, None


def _run_route_collector(args, output: Path, request: dict | None, gap_reason: str | None) -> dict:
    """Invoke the collector while the ordinary M/page/process remain live."""

    collector = ROOT / "scripts/android_pair_route_receipt_current.py"
    route_output = output / "private-route-receipt.json"
    command = ["rtk", "proxy", "python3", str(collector), "--output", str(route_output)]
    if request is None:
        command += ["--gap-reason", gap_reason or "native-live-m-observer-unavailable"]
    else:
        request_path = output / "private-route-request.json"
        _private_write(request_path, request)
        command += ["--request", str(request_path)]
    executed = subprocess.run(command, cwd=ROOT, capture_output=True, text=True)
    summary = None
    if executed.stdout.strip():
        try:
            summary = json.loads(executed.stdout.strip().splitlines()[-1])
        except ValueError:
            summary = None
    return {"commandExit": executed.returncode, "receipt": str(route_output),
            "summary": summary, "gapReason": gap_reason,
            "stderrPresent": bool(executed.stderr.strip())}


def _route_summary(route_observation: object) -> dict:
    """Return a safe route summary when collection was skipped or failed."""

    if not isinstance(route_observation, dict):
        return {}
    summary = route_observation.get("summary")
    return summary if isinstance(summary, dict) else {}


def _finalize_m82_report(report: dict, current_runtime) -> dict:
    """Finalize qualification without masking the story or cleanup failure."""

    cleanup = report.setdefault("cleanup", {})
    browser_cleanup_ok = False
    browser_cleanup = cleanup.get("browser")
    if isinstance(browser_cleanup, dict):
        try:
            browser_cleanup_ok = current_runtime.browser_cleanup_succeeded(browser_cleanup)
        except Exception as error:
            cleanup["browserQualificationFailure"] = {
                "failureClass": type(error).__name__, "message": str(error)}
    report["passed"] = bool(
        report.get("passed") is True
        and cleanup.get("nativeQualified") is True
        and browser_cleanup_ok
        and cleanup.get("packetCapture", {}).get("cleanupSucceeded", True) is True)
    summary = _route_summary(report.get("routeObservation"))
    report["physicalDataQualified"] = bool(
        report.get("passed") is True
        and report.get("normalTlsVerified") is True
        and summary.get("physicalDataQualified") is True)
    report["physicalWanQualified"] = report["physicalDataQualified"]
    return report


def _read_m82_rtc_progress(page, timeout: float = 1.0) -> dict | None:
    """Drain the latest private, redacted RTC progress snapshot."""

    deadline = time.monotonic() + timeout
    quiet_until = None
    latest = None
    with page.condition:
        while time.monotonic() < deadline:
            reply = page.responses.pop("mio82-rtc-progress", None)
            if isinstance(reply, dict) and reply.get("ok") is True \
                    and isinstance(reply.get("result"), dict):
                latest = reply["result"]
                quiet_until = time.monotonic() + 0.1
            now = time.monotonic()
            if latest is not None and quiet_until is not None and now >= quiet_until:
                return latest
            page.condition.wait(min(0.05, max(0.001, deadline - now)))
    return latest


def run_m82_story(args, native_guard: dict, browser_guard: dict, current_runtime, frozen) -> int:
    """M82 admission/reopen/nonempty replica/live ordinary-M story."""

    from check_browser_relay_capability import OwnedCdpObserver
    from check_unified_browser_native import close_native, native_cleanup_succeeded

    binary = Path(native_guard["deployedPath"])
    facility = json.loads(args.facility.read_text())
    browser_profile = json.loads(args.browser_profile.read_text())
    args.output.mkdir(mode=0o700, parents=True)
    report = {"scope": "M82 restricted admission/Complete/reopen/nonempty replica/live ordinary-M RTC PING/PONG",
              "currentRuntimeSelection": current_runtime.runtime_selection(),
              "currentNativePlatform": current_runtime.runtime_selection()["native"]["platform"],
              "currentBrowserPlatform": current_runtime.runtime_selection()["browser"]["platform"],
              "currentArtifactGuard": native_guard, "currentBrowserArtifactGuard": browser_guard,
              "normalTlsVerified": False, "physicalDataQualified": False,
              "physicalWanQualified": False, "passed": False, "stages": [],
              "routeObservation": None, "cleanup": {}, "firstFailure": None}
    private = {"sdkReplies": [], "nativeEvents": [], "nativeReplies": [], "ownership": {}}
    actor = page = observer = packet_capture = None
    native_device = browser_device = None
    browser_id = None
    member_native = None

    def save_private():
        _private_write(args.output / "private-m82-transcript.json", private)

    def save():
        _private_write(args.output / "receipt.json", report)

    def ownership_snapshot():
        """Retain exact owned cleanup handles only in the 0600 transcript."""

        ownership = {}
        if actor is not None:
            directory = actor.directory
            ownership["native"] = {
                "model": current_runtime.runtime_selection()["native"]["model"],
                "transport": current_runtime.runtime_selection()["native"]["transport"], "directory": directory,
                "remotePid": actor.remote_pid,
                "adbCommands": [entry for entry in actor.device.commands
                                if directory in json.dumps(entry, sort_keys=True)],
            }
        if page is not None:
            ownership["browser"] = {
                "model": "Pixel_6a", "transport": "usb", "actor": page.actor,
                "url": page.url, "forward": page.forward, "reverse": page.reverse,
                "adbCommands": [entry for entry in page.device.commands
                                if any(kind in entry.get("command", [])
                                       for kind in ("forward", "reverse"))],
            }
        private["ownership"] = ownership
        return ownership

    def fail(message):
        raise CurrentGuardError(message)

    def stage(name, operation):
        started = time.monotonic()
        try:
            value = operation()
            report["stages"].append({"name": name, "ok": True,
                                     "elapsedMs": round((time.monotonic() - started) * 1000, 3)})
            save()
            return value
        except Exception as error:
            if report.get("firstFailure") is None:
                report["firstFailure"] = {"phase": name, "failureClass": type(error).__name__}
            report["stages"].append({"name": name, "ok": False,
                                     "failureClass": type(error).__name__})
            save()
            raise

    def sdk(operation, values=None):
        try:
            # Observe the current 60s SDK caller through its real return. This
            # RPC wait is separate from the SDK's unchanged absolute deadline.
            reply = page.call(operation, values, timeout=65)
        except Exception as error:
            failure = {"phase": "sdk:" + operation,
                       "failureClass": type(error).__name__,
                       "message": str(error)}
            private["sdkReplies"].append({"operation": operation, "exception": failure})
            save_private()
            if report.get("firstFailure") is None:
                report["firstFailure"] = failure
            report.setdefault("sdkFailures", []).append(failure)
            save()
            raise
        private["sdkReplies"].append({"operation": operation, "reply": reply})
        save_private()
        report.setdefault("sdkCalls", []).append({"operation": operation,
                                                    "ok": reply.get("ok") is True,
                                                    "callerElapsedMs": reply.get("callerElapsedMs")})
        save()
        return reply

    def preserve_native_observation(phase):
        try:
            if actor is None:
                return
            with actor.condition:
                events = list(actor.events[-64:])
                truncated = len(actor.events) > len(events)
                stderr = list(actor.stderr[-64:])
            # Read the existing bounded actor buffers only. No SDK command or
            # timer is added, and raw events stay in the private transcript.
            encoded = json.dumps(events)
            if len(encoded.encode()) > 1024 * 1024:
                events = []
                truncated = True
            private.setdefault("nativeUnconsumedObservations", []).append({
                "phase": phase, "events": events, "stderr": stderr,
                "truncated": truncated, "maxEvents": 64, "maxBytes": 1024 * 1024,
            })
            save_private()
        except Exception as error:
            report.setdefault("nativeObserverFailures", []).append({
                "phase": phase, "failureClass": type(error).__name__,
            })
            # Observation failure must not replace the SDK result or skip
            # the existing owned-resource cleanup.

    def checked(reply, operation):
        if reply.get("ok") is not True:
            if operation == "open_member" and page is not None:
                preserve_native_observation("ordinary-member-rejected")
                progress = _read_m82_rtc_progress(page)
                if progress is not None:
                    private["rtcProgress"] = progress
                    report["rtcProgress"] = {key: progress.get(key)
                                              for key in ("instrumented", "sequence", "peerCount",
                                                          "dataChannelCount", "closeCount")}
                    save_private()
            failure = {"phase": "sdk:" + operation, "failureClass": "SdkReplyFailure"}
            if isinstance(reply, dict) and reply.get("failure") is not None:
                failure["failure"] = str(reply["failure"])
            if report.get("firstFailure") is None:
                report["firstFailure"] = failure
            report.setdefault("sdkFailures", []).append(failure)
            save()
            fail("browser SDK operation failed: " + operation)
        return reply.get("result")

    def event(value):
        private["nativeEvents"].append(value)
        save_private()
        return value

    def native(operation, expected, **values):
        private["nativeReplies"].append({"operation": operation, "values": values})
        # The current operator ABI is JSON-line without an RPC id.  Sending a
        # dict through NativeActor.command would add an id and wait forever for
        # a reply field the operator never emits.
        value = actor.command(json.dumps({"op": operation, **values}), expected)
        private["nativeReplies"][-1]["reply"] = value
        save_private()
        return event(value)

    def configuration(ready):
        return {"relays": [args.relay] if args.relay else [], "rtc_enabled": not args.rtc_disabled,
                "disabled": [], "irc": [], "mqtt": [facility], "rtc_ice": browser_profile,
                "contact_routes": [{"contact": ready["id"], "facility": facility["logical_id"],
                                    "address": ready["route"]}]}

    try:
        native_device = stage("native-device-selection", lambda: current_runtime.device_for("native"))
        actor = stage("owned-native-actor", lambda: current_runtime.NativeActor(native_device))
        ownership_snapshot()
        save_private()
        actor.deploy(binary, "admission-story", executable=True)
        facility_remote = actor.deploy(args.facility, "facility.json")
        profile_remote = actor.deploy(args.native_profile, "ice-profile.json")
        data_dir = actor.directory + "/saved-provider"
        native_args = [data_dir, facility_remote, profile_remote, "--allow-unqualified-preparation"]
        if args.relay:
            native_args += ["--relay", args.relay]
        if args.rtc_disabled:
            native_args += ["--rtc-disabled"]
        if args.packet_capture_helper:
            import importlib.util
            spec = importlib.util.spec_from_file_location("m82_owned_packet_capture", args.packet_capture_helper)
            capture_module = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(capture_module)
            packet_capture = capture_module.CaptureGroup(
                args.output / "private-pcap", facility, json.loads(args.native_profile.read_text()))
            stage("native-packet-capture-ready", lambda: packet_capture.start_device("native", native_device))
        actor.start(native_args)
        ready = stage("native-sdk-start", lambda: event(actor.wait_event("ready")))
        if ready.get("normalPublicTrust") is not True:
            fail("native preparation contract differs")
        traffic = ready.get("traffic", {}) if isinstance(ready, dict) else {}
        protocol = traffic.get("protocol", {}) if isinstance(traffic, dict) else {}
        # The current fixture uses the SDK's normal public-root TLS path.  A
        # route is only marked TLS-verified when the configured native entry is
        # mqtts and the live MQTT binding reports CONNACK/SUBACK success.
        report["normalTlsVerified"] = bool(
            str(facility.get("native_url", "")).startswith("mqtts://")
            and isinstance(protocol, dict)
            and protocol.get("connack") in (0, 1)
            and protocol.get("suback_codes") == [0])
        browser_device = stage("browser-device-selection", lambda: current_runtime.device_for("browser"))
        if packet_capture:
            stage("browser-packet-capture-ready", lambda: packet_capture.start_device("browser", browser_device))
        page = current_runtime.BrowserPage(browser_device, args.package, prelude=_m82_prelude(),
                                           environment_rtc_probe=False)
        stage("owned-browser-page", page.start)
        if packet_capture:
            packet_capture.observe(actor, page)
        ownership_snapshot()
        save_private()
        rtc_progress = stage("m82-rtc-observer-ready", lambda: _read_m82_rtc_progress(page))
        if not isinstance(rtc_progress, dict) or rtc_progress.get("instrumented") is not True:
            fail("M82 RTC progress observer did not install")
        private["rtcProgress"] = rtc_progress
        report["rtcProgress"] = {key: rtc_progress.get(key)
                                  for key in ("instrumented", "sequence", "peerCount",
                                              "dataChannelCount", "closeCount")}
        save_private()
        fixture = stage("m82-admission-fixture-ready", lambda: frozen.fixture_facts(
            page, lambda value: isinstance(value, dict) and value.get("faultArmed") is False))
        report["admissionCommitFixture"] = {
            "abortSecretOnce": False,
            "faultArmed": fixture.get("faultArmed"),
            "faultDisabled": fixture.get("faultArmed") is False,
            "scope": fixture.get("scope"),
        }
        observer = OwnedCdpObserver(page)
        observer.start()
        first = stage("browser-sdk-start", lambda: checked(sdk("start", {"configuration": configuration(ready)}), "start"))
        browser_id = first["id"]
        if browser_id == ready["id"]:
            fail("browser and Native identities are not independent")
        request = stage("restricted-admission-request", lambda: checked(
            sdk("request_admission", {"invitation": ready["invitation"]}), "request_admission"))
        if request != "AwaitingApproval":
            fail("restricted admission did not remain AwaitingApproval")
        accepted = stage("native-restricted-admission", lambda: event(actor.wait_event("admission-accepted")))
        if accepted.get("restricted") is not True or accepted.get("businessHandle") is not False:
            fail("admission released business permission")
        pending = native("admission-status", "admission-status")
        requests = [value for value in pending.get("requests", []) if value.get("applicant") == browser_id]
        if len(requests) != 1:
            fail("exact browser admission request unavailable")
        probe = stage("different-TLS-identity-claim", lambda: sdk("admission_identity_probe", {
            "request": requests[0]["request"], "claimed": ready["id"]}))
        probe_accepted = stage("native-TLS-claim-restricted", lambda: event(actor.wait_event(
            lambda value: value.get("event") == "admission-accepted"
            and value.get("stableId") != accepted.get("stableId"))))
        probe_ended = stage("native-TLS-claim-rejected", lambda: event(actor.wait_event(
            lambda value: value.get("event") == "admission-ended"
            and value.get("stableId") == probe_accepted.get("stableId"))))
        after_probe = native("admission-status", "admission-status")
        if (probe.get("ok") is not False or probe.get("failure") != "connection lost"
                or probe.get("result") is not None or probe_accepted.get("restricted") is not True
                or probe_accepted.get("businessHandle") is not False
                or probe_accepted.get("alpn") != frozen.STRICT_ADMISSION_ALPN
                or probe_ended.get("succeeded") is not False
                or probe_ended.get("providerFailureClass") != "Rejected"
                or after_probe.get("requests") != pending.get("requests")
                or after_probe.get("appliedCount") != pending.get("appliedCount")):
            fail("different TLS identity claim was not canonically rejected without ledger mutation")
        report["wrongTlsIdentity"] = {"canonicalRejected": True, "noApprovedMaterial": True,
                                      "restrictedNewPurpose": True, "noLedgerMutation": True}
        for operation in ("open_member", "sync_replica"):
            stage("unapproved-" + operation + "-rejected", lambda operation=operation:
                  frozen.require_unknown_data_rejection(sdk(operation, {"target": ready["id"]})))
        report["unapprovedDataRejected"] = True
        native("approve-request", "approved", applicant=browser_id, request=requests[0]["request"])
        if checked(sdk("request_admission", {"invitation": ready["invitation"]}), "request_admission") != "PendingMaterials":
            fail("approved admission did not expose PendingMaterials")
        authorization = checked(sdk("material_receipt"), "material_receipt")
        if authorization.get("authorization_saved") is not True or authorization.get("secret_saved") is not False:
            fail("authorization material boundary was not observed")
        stage("authorization-only-stop", lambda: _require_browser_stop(sdk, checked))
        reopened = stage("authorization-only-reopen", lambda: checked(
            sdk("start", {"configuration": configuration(ready)}), "start"))
        if reopened.get("id") != browser_id:
            fail("browser identity changed during durable reopen")
        if checked(sdk("request_admission", {"invitation": ready["invitation"]}), "request_admission") != "PendingMaterials":
            fail("same durable admission grant did not resume")
        resumed = checked(sdk("material_receipt"), "material_receipt")
        if not frozen.same_grant(authorization, resumed):
            fail("admission grant changed during reopen")
        complete_reply = checked(sdk("complete_admission"), "complete_admission")
        if complete_reply != "Complete":
            fail("durable Complete was not observed")
        complete = checked(sdk("material_receipt"), "material_receipt")
        if complete.get("authorization_saved") is not True or complete.get("secret_saved") is not True:
            fail("Complete material receipt is incomplete")
        stage("complete-reopen-stop", lambda: _require_browser_stop(sdk, checked))
        author = stage("independent-offline-author", lambda: native("offline-author", "offline-author"))
        if (author.get("independentIdentity") is not True or author.get("providerVerifiedApplied") is not True
                or author.get("bodyPresent") is not False or author.get("serializedRecordBytes", 0) <= 0
                or author.get("authorCleanup", {}).get("ownedSocketsAbsent") is not True
                or author.get("authorCleanup", {}).get("ownedSocketsAbsentLate") is not True):
            fail("independent offline SDK author did not persist and retire")
        target_record = author["record"]
        report["offlineAuthor"] = {key: author[key] for key in (
            "independentIdentity", "serializedRecordBytes", "bodyPresent", "bodyBytes",
            "providerVerifiedApplied", "authorCleanup", "savedProviderResourceConfigurationRestored")}
        provider_retired = stage("saved-provider-retirement", lambda:
                                 close_native(actor, remove_directory=False))
        report["providerReopenCleanup"] = provider_retired
        if not native_cleanup_succeeded(provider_retired, False):
            fail("saved provider did not retire before reopening")
        actor.start(native_args)
        ready_again = stage("saved-provider-reopen", lambda: event(actor.wait_event("ready")))
        if ready_again.get("id") != ready["id"]:
            fail("saved provider identity changed during reopen")
        ready = ready_again
        final_reopen = stage("complete-reopen", lambda: checked(
            sdk("start", {"configuration": configuration(ready)}), "start"))
        if final_reopen.get("id") != browser_id or not frozen.same_grant(complete, checked(sdk("material_receipt"), "material_receipt")):
            fail("Complete identity/grant changed after final reopen")
        before_replica = checked(sdk("material_receipt"), "material_receipt")
        replica = stage("nonempty-sdk-replica", lambda: checked(
            sdk("sync_replica", {"target": ready["id"]}), "sync_replica"))
        sender = stage("sender-durable-replica-receipt", lambda: event(actor.wait_event("replica-finished")))
        replica_diagnostics = checked(sdk("replica_diagnostics"), "replica_diagnostics")
        stage("strict-new-replica-purpose", lambda: frozen.require_replica_wire(replica_diagnostics, sender))
        report["replica"] = frozen.require_replica(replica, target_record, before_replica,
                                                  checked(sdk("material_receipt"), "material_receipt"))
        stage("replica-reopen-stop", lambda: _require_browser_stop(sdk, checked))
        replica_reopen = stage("replica-reopen", lambda: checked(
            sdk("start", {"configuration": configuration(ready)}), "start"))
        persisted = checked(sdk("material_receipt"), "material_receipt")
        if (replica_reopen.get("id") != browser_id or not frozen.same_grant(complete, persisted)
                or target_record not in persisted.get("records", [])):
            fail("verified nonempty replica did not survive browser reopen")
        report["replica"]["targetVerifiedSavedAfterReopen"] = True
        stage("ordinary-member-open", lambda: checked(sdk("open_member", {"target": ready["id"]}), "open_member"))
        member_diagnostics = checked(sdk("member_diagnostics"), "member_diagnostics")
        member_native = stage("native-live-member", lambda: event(actor.wait_event("member-released")))
        stable, browser_stable = _live_member_local_ids(member_native, member_diagnostics, browser_id)
        paths = member_diagnostics.get("paths", []) if isinstance(member_diagnostics, dict) else []
        if not any(path.get("selected") is True and path.get("isRtc") is True for path in paths):
            fail("browser selected RTC path was not observed on the live ordinary M")
        bootstrap = member_diagnostics.get("bootstrap", {}) if isinstance(member_diagnostics, dict) else {}
        coordinator = member_diagnostics.get("coordinator", {}) if isinstance(member_diagnostics, dict) else {}
        if not isinstance(coordinator, dict) or not coordinator:
            coordinator = bootstrap.get("coordinator", {}) if isinstance(bootstrap, dict) else {}
        if not isinstance(coordinator, dict) or coordinator.get("freshAck") is not True:
            fail("browser current member gate lacks a fresh final ACK")
        ping = checked(sdk("exchange", {"message": "PING"}), "exchange")
        if ping != "PONG":
            fail("SDK PING/PONG exchange failed")
        business = stage("native-live-business", lambda: event(actor.wait_event("business")))
        if business.get("stableId") != stable or business.get("memberMessage") is not True \
                or business.get("reply") != "PONG" or business.get("bytes") != 4:
            fail("business response was not on the original M")
        member_after = checked(sdk("member_diagnostics"), "member_diagnostics")
        _, browser_after_stable = _live_member_local_ids(member_native, member_after, browser_id)
        if browser_after_stable != browser_stable:
            fail("browser post-business diagnostics changed ordinary M")
        browser_path_diagnostics = checked(sdk("path_diagnostics"), "path_diagnostics")
        report["normalTlsVerified"] = bool(
            report.get("normalTlsVerified") and _browser_normal_tls(browser_path_diagnostics))
        if not any(path.get("selected") is True and path.get("isRtc") is True for path in member_after.get("paths", [])):
            fail("selected RTC path did not remain live after PING/PONG")
        after_coord = member_after.get("coordinator", {})
        if not isinstance(after_coord, dict) or not after_coord:
            after_coord = member_after.get("bootstrap", {}).get("coordinator", {})
        if not isinstance(after_coord, dict) or after_coord.get("freshAck") is not True:
            fail("browser current member gate lost its fresh final ACK")
        mark_reply = None
        try:
            mark_reply = native("member-socket-mark", lambda value: value.get("event") == "member-socket-mark",
                                stableId=stable)
        except Exception:
            report["liveTupleObservationGap"] = "native operator member-socket-mark ABI unavailable"
        packet_witness = packet_capture.snapshot() if packet_capture is not None else None
        route_request, gap = _route_request_from_live_m(member_native, member_after,
                                                        mark_reply, actor.directory, packet_witness)
        if args.native_platform == "android":
            report["routeObservation"] = _run_route_collector(args, args.output, route_request, gap)
        else:
            from host_pixel_route_receipt_current import collect as collect_host_pixel
            route_path = args.output / "private-host-route-receipt.json"
            route_value = {"summary": {"physicalDataQualified": False, "physicalWanQualified": False},
                           "gapReason": gap or "host-upstream-route-qualification-separate"}
            try:
                if route_request is not None:
                    route_request["endpoints"][0].update(model="LinuxHost", transport="local")
                    route_value = collect_host_pixel(route_request, actor, browser_device)
            except Exception as error:
                route_value["observerFailure"] = {"failureClass": type(error).__name__,
                                                  "message": str(error)[:256]}
            _private_write(route_path, route_value)
            report["routeObservation"] = {"scope": "Host/Linux and Pixel route observer",
                "receipt": str(route_path), "summary": route_value["summary"],
                "gapReason": route_value.get("gapReason", "host-upstream-route-qualification-separate")}
        stage("ordinary-member-close", lambda: checked(sdk("close_member"), "close_member"))
        removed = stage("actual-current-member-removal", lambda: native("remove-member", "removed", target=browser_id))
        if removed.get("currentAccessAbsent") is not True:
            fail("actual member removal did not revoke access")
        for operation in ("open_member", "sync_replica"):
            stage("revoked-" + operation + "-rejected", lambda operation=operation:
                  frozen.require_unknown_data_rejection(sdk(operation, {"target": ready["id"]})))
        report["revokedDataRejected"] = True
        report["m82StoryQualified"] = True
        report["passed"] = True
    except Exception as error:
        private["storyFailure"] = {"failureClass": type(error).__name__, "message": str(error)[:512]}
        if report.get("firstFailure") is None:
            report["firstFailure"] = {"phase": "m82-story", "failureClass": type(error).__name__}
        report["failureClass"] = type(error).__name__
    finally:
        preserve_native_observation("before-owned-cleanup")
        if observer is not None:
            try:
                report["cleanup"]["browserObserver"] = observer.close()
            except Exception as error:
                report["cleanup"]["browserObserver"] = {"failureClass": type(error).__name__}
        if page is not None:
            try:
                report["cleanup"]["browser"] = page.close()
            except Exception as error:
                report["cleanup"]["browser"] = {"failureClass": type(error).__name__}
        if actor is not None:
            try:
                cleanup = close_native(actor)
                report["cleanup"]["native"] = cleanup
                report["cleanup"]["nativeQualified"] = native_cleanup_succeeded(cleanup)
            except Exception as error:
                report["cleanup"]["native"] = {"failureClass": type(error).__name__}
                report["cleanup"]["nativeQualified"] = False
            preserve_native_observation("after-owned-native-close")
        if packet_capture is not None:
            try:
                captured = packet_capture.close()
                private["packetCapture"] = captured
                report["cleanup"]["packetCapture"] = {
                    "cleanupSucceeded": captured.get("cleanupSucceeded") is True,
                    "observerJoined": captured.get("observerJoined") is True,
                    "observerFailureCount": len(captured.get("observerFailures", [])),
                    "firstPeerCheckCoveredByExpandedFilterProven": False,
                    "devices": {
                        role: {
                            "ownedDirectoryAbsent": value.get("ownedDirectoryAbsent"),
                            "cleanupSucceeded": value.get("cleanupSucceeded"),
                            "segments": [{key: item.get(key) for key in (
                                "name", "ready", "startedBeforeSdk", "forced", "exit", "readersEnded",
                                "ownedProcessAbsentImmediate", "ownedProcessAbsentLate", "pcapBytes",
                                "pcapSha256", "packetsCaptured", "packetsReceivedByFilter", "kernelDrops", "limitReached",
                            )} for item in value.get("records", [])],
                        } for role, value in captured.get("devices", {}).items()
                    },
                }
            except Exception as error:
                report["cleanup"]["packetCapture"] = {"cleanupSucceeded": False,
                    "failureClass": type(error).__name__}
        _finalize_m82_report(report, current_runtime)
        private["cleanup"] = json.loads(json.dumps(report.get("cleanup", {})))
        ownership_snapshot()
        private["ownershipAfterCleanup"] = json.loads(json.dumps(private.get("ownership", {})))
        save_private()
        save()
    return 0 if report.get("passed") else 1


def _require_browser_stop(sdk, checked):
    value = checked(sdk("stop"), "stop")
    if value.get("cleanupSucceeded") is not True or value.get("networkClosed") is not True \
            or value.get("networkFreed") is not True or value.get("ownedMemberHandles") != 0:
        raise CurrentGuardError("browser SDK stop did not retire owned network")


def _parse_args(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--development-checks", action="store_true")
    parser.add_argument("--execute", action="store_true")
    parser.add_argument("--allow-unqualified-preparation", action="store_true")
    parser.add_argument("--require-strict-purpose", action="store_true")
    parser.add_argument("--packet-capture-helper", type=Path)
    parser.add_argument("--relay")
    parser.add_argument("--rtc-disabled", action="store_true")
    parser.add_argument("--native-platform", choices=("android",), default="android")
    parser.add_argument("--native-build-receipt-sha256")
    parser.add_argument("--admission-old-wire-observation", type=Path)
    for name in ("output", "build-receipt", "wasm-build-receipt", "package", "facility",
                 "native-profile", "browser-profile"):
        parser.add_argument("--" + name, type=Path)
    return parser.parse_args(argv)


def _development_checks() -> dict:
    """Meaningful parser/selection counterexamples; no ADB or SDK operation."""

    try:
        from android_pair_runtime_current import CURRENT_DEVICES, LEGACY_NATIVE_CALL
    except ModuleNotFoundError:
        from scripts.android_pair_runtime_current import CURRENT_DEVICES, LEGACY_NATIVE_CALL
    require(CURRENT_DEVICES["native"] == {
        "model": "PJZ110", "transport": "usb", "platform": "Android/PJZ110"},
            "native current selection changed")
    require(CURRENT_DEVICES["browser"]["model"] == "Pixel_6a"
            and CURRENT_DEVICES["browser"]["transport"] == "usb",
            "browser current selection changed")
    require(LEGACY_NATIVE_CALL == ("TB322FC", "network"),
            "legacy adapter sentinel changed")
    return {"adbStarted": False, "actorsStarted": False, "currentSelection": True,
            "legacyAliasTranslatedOnlyAtSeam": True, "physicalWanQualified": False}


def main(argv=None) -> int:
    args = _parse_args(argv)
    if args.development_checks:
        require(not args.execute, "development checks and device execution are separate")
        print(json.dumps(_development_checks(), indent=2))
        return 0
    require(args.execute and args.allow_unqualified_preparation,
            "explicit current Android candidate execution selection is required")
    required = ("output", "build_receipt", "wasm_build_receipt", "package", "facility",
                "native_profile", "browser_profile")
    require(all(getattr(args, name) for name in required),
            "all current artifacts and explicit resource profiles are required")
    require(not args.output.exists(), "output exists; preserve the original receipt")
    native_guard = guard_current_android_build(args.build_receipt,
                                               args.native_build_receipt_sha256)
    browser_guard = _guard_frozen_browser_build(args.wasm_build_receipt, args.package)

    # No actor is created before the source/artifact/depfile/config guards
    # above have passed.  The legacy controller is imported only for its
    # private DTO/cleanup primitives; its full M83 run() is never called.
    current_runtime = importlib.import_module("android_pair_runtime_current")
    current_runtime.install_for_frozen_controller()
    frozen = importlib.import_module("check_unified_admission")
    return run_m82_story(args, native_guard, browser_guard, current_runtime, frozen)


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as error:
        print(json.dumps({"commandExit": 1, "firstFailure": {
            "phase": "current-controller-preflight", "failureClass": type(error).__name__,
            "message": str(error)}}))
        raise SystemExit(1)
