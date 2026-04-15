# 2026-04-15 Alpha Product Discussion Notes

> Status: temporary discussion record only.
> This is not a charter, not a roadmap commitment, and not a finalized product spec.

## Background

Current `miopunch` progress is still centered on punching/connectivity/dataplane capabilities.
That is not enough to count as a meaningful product POC.
The next stage should move from "can punch through" toward "can be used for something real".

This round of discussion is about product direction, alpha scope, and what kind of
user-facing capability should sit on top of the existing connectivity work.

## What Is Already Clear

- Product direction is no longer "just a punching demo".
- Target direction is a cross-platform private direct-connect networking product.
- The product should be usable by individuals and small teams.
- A major distinguishing goal is high interpretability / explainability.
- The product should make runtime state understandable instead of hiding everything.
- Users should be able to understand most of the current state at a glance.
- `web` cannot replace the native client.
- Native clients remain required on Windows / Linux / Android.
- iOS is not in current scope.
- Android may later get a native rewrite, but that is not required for the current stage.
- A local HTTP server + control page is acceptable and useful, but it is only part of the product.
- Encryption should follow industry best practices, while avoiding unnecessary complexity.

## Direction Shift Agreed In Principle

The project should evolve toward something closer to:

- a private direct-connect mesh / networking tool,
- with strong runtime visibility,
- and at least one real user-facing capability above the current connectivity layer.

In other words, the next meaningful milestone should not stop at link establishment.
It must expose a practical capability that helps users actually do something.

## Capability Ideas Discussed

### Rejected / Not Preferred As Primary Alpha Capability

- "Just show punching succeeded" is not enough.
- "Access remote HTTP service" is not preferred as the first main capability.
- File transfer is acceptable, but feels only barely sufficient as the first product-facing capability.

### Stronger Candidate: Shell / CLI Access

A stronger candidate direction is remote shell passthrough / relay.
The motivating scenario is:

- a user is outside the home network,
- wants to access a machine at home,
- and wants to use shell-based tools such as Codex or Claude Code CLI remotely,
- ideally in a way that remains usable even on unstable mobile environments such as 5G or high-speed rail.

This direction feels more like a real product than a pure punching demo.
It also better matches the current era of CLI-based AI workflows.

## Current Layering Understanding

Current implementation is understood as covering roughly these lower layers:

- signaling / exchange,
- connectivity / punching,
- dataplane transport.

What is still missing is a product-facing layer above them.
That product-facing layer likely includes:

- device identity / pairing,
- network membership,
- capability exposure,
- runtime explanation,
- and at least one real end-user function.

## Pairing / Key Exchange Idea Mentioned

One idea raised in discussion:

- provide a web page or similar helper that generates a local keypair,
- keep key generation local rather than tying it to the author's server,
- use QR code or similar out-of-band interaction to help devices pair.

This idea is still only exploratory.
It is not yet a settled architecture.

## Points Still Not Settled

- exact product statement and wording;
- how much of the terminal experience should be browser-based versus native-client-based;
- whether file transfer belongs in the first alpha or only after shell access is working well;
- how pairing, identity, and session authorization should be shaped;
- how much center-assisted signaling is acceptable in practice;
- how to present complex punching/NAT/path-selection details in a way ordinary users can understand quickly.

## Temporary Working Summary

For now, the safest temporary summary is:

- `miopunch` is moving toward a cross-platform private direct-connect networking product;
- the product should emphasize explainability and low user cognitive burden;
- native clients remain the core entry point;
- local web UI can be an important control/explanation surface but does not replace the client;
- the next real milestone should be a usable alpha capability above connectivity;
- remote shell / CLI access is the chosen alpha POC capability.

## Suggested Next Discussion Topics

1. Define the concrete CLI/UI user flows for the alpha shell POC.
2. Define target discovery + selection UX for `wsl:<distro>` and `ssh:<name>`.
3. Define what “explainability” means in UI/events.
4. Decide what should remain temporary and what is stable enough to enter a formal charter.

## Alpha POC Scope (Agreed)

- Primary POC capability: **remote shell (PTY relay)**.
- **Controlled nodes**: Windows host, Linux.
- **Controller nodes**: Windows / Linux / Android.
- **Android is controller-only** (no Android controlled/agent mode in this stage).
- End-to-end encryption is required for both exchange (MQTT) and payload;
  use industry best practices and Go stdlib primitives (no custom crypto designs).

### Windows Targets (Agreed)

- WSL2 / WSL distros provide the shell via `tmux`.
- Local VMs provide the shell via `tmux`.
- `miopunch` runs on the Windows host (real NIC), not inside each WSL/VM.

### Target Connectors (Agreed)

- **WSL connector (default)**: ConPTY + `wsl.exe` (SSH-like interactive access without requiring `sshd` inside WSL).
- **VM connector**: `ssh` (assumes SSH access to the VM).
- Windows native `powershell` / `cmd` targets are out of scope for this POC.

### Session Persistence Expectations

- A shell session should persist until the user explicitly closes it.
- The expected experience is "like it keeps running on the target machine".
- The client device may close/reboot; after coming back, the user can resume the same session.
- Persistence/resume is implemented by anchoring the session in target-side `tmux`.
- Control-side disconnects and controlled-side agent restarts must not lose the session;
  resuming is implemented by re-attaching to the same `tmux` session.
- `miopunch` does not implement its own multiplexer; it relies on `tmux`.

### Two Agent Usage Modes

- **Owned devices**: run a resident agent (always-on) for best UX.
- **Temporary devices**: download the binary, run a one-shot/temporary agent with explicit key/authorization,
  create a temporary session, and fully exit after the user leaves.

### Target Advertisement and Shortcuts

- The target side should advertise which shell targets are available.
- POC: Windows targets are `wsl:<distro>` and `ssh:<name>`.
- Users should be able to configure additional shortcuts, e.g. an `ssh` jump to a local VM, exposed as a named target.

### Resume Semantics (Agreed)

- "Restore the scene" means re-attaching to the same `tmux` session.
- No replay/snapshot logic is planned for this stage.

### UX Principles

- CLI-first flow must exist: run the binary in any terminal, provide minimal inputs, enter the remote shell.
- UI is still valuable for explainability, session list, and "resume" discovery.
- Desktop UI should ideally open the user's preferred terminal app when attaching to a session.
