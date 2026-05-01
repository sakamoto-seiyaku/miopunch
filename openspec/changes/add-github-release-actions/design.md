## Context

The root repository currently has no `.github/workflows` directory and no root release automation. Existing packaging scaffolds already exist under `packaging/linux/deb` and `packaging/windows/nsis`; the release change should reuse them rather than invent a parallel packaging path.

The intended first tag is `v0.1.0-rc.1`. The local `origin` remote is intentionally a local SSH mirror endpoint; the maintainer will push tags there and let the mirror chain synchronize to GitHub.

Current implementation notes discovered during planning:

- Linux desktop builds can compile with `go build -tags desktop ./cmd/miopunch-desktop`.
- Windows cross-build currently fails in `connectivity/tcp_reuse_windows.go`; the apply phase must fix that before Windows assets can be produced.
- Lab execution can run without `/dev/kvm` by falling back to QEMU TCG, but this can be slow on hosted runners.

## Goals / Non-Goals

**Goals:**

- Add split GitHub Actions workflows for build, host checks, core lab gates, scenario lab gates, and release publishing.
- Make release publishing depend on required host checks, artifact builds, and core lab gates.
- Keep scenario 1/2/3 gates runnable as a standalone diagnostic workflow while treating them as local pre-release operator validation.
- Produce the full v0 desktop release surface: raw binary bundles, Linux `.deb` variants, Windows NSIS installer, checksums, manifest, and attestations.
- Keep tag ownership manual and explicit.

**Non-Goals:**

- Do not create or push Git tags from CI.
- Do not add Docker image publishing, Homebrew, RPM, macOS artifacts, store distribution, or signing certificate workflows.
- Do not replace the existing lab harness or packaging scripts unless a small compatibility fix is needed for CI.

## Decisions

### Split workflows by responsibility

Use these workflow boundaries:

- `go-checks.yml`: host Go gates and cross-build sanity.
- `build-artifacts.yml`: compile/package assets and upload temporary Actions artifacts.
- `lab-core-gates.yml`: legacy/core lab gates.
- `lab-scenarios.yml`: MNT-01, MNT-02, and MNT-03 scenario gates.
- `release.yml`: tag-triggered release orchestration and GitHub Release publishing.

Rationale: the user explicitly wants separate workflows, and split workflows make slow lab gates diagnosable without rebuilding or republishing.

Alternative considered: one large release workflow. Rejected because it mixes fast build failures with slow VM/lab failures and makes manual reruns less targeted.

### Release workflow reuses the same commands as standalone release-blocking workflows

The apply phase should avoid copying divergent shell logic between standalone and release paths. Prefer reusable workflow calls or shared scripts where GitHub Actions syntax makes that practical. If reusable workflows are too awkward for artifact fan-in, keep command sequences aligned and documented in comments.

Rationale: release-blocking gates must mean the same thing as the independently runnable workflows they call.

Alternative considered: release workflow checks only status of prior branch workflows. Rejected because a tag must be self-contained and reproducible at the tagged commit.

### GitHub-hosted Ubuntu is the v0 core lab runner

Use GitHub-hosted Ubuntu runners for release-blocking core lab workflows as requested. Install host dependencies in the workflow, cache the Debian cloud image, and upload `lab/_artifacts/`, `lab/_state/qemu.log`, and `lab/_state/serial.log` on completion or failure. Keep the scenario workflow available for manual diagnostics, but do not make release publishing depend on scenario 1/2/3 because GitHub-hosted runners do not reliably complete them.

Rationale: this keeps the first GitHub setup self-contained and avoids requiring a self-hosted runner before the first release.

Alternative considered: dedicated self-hosted KVM runner. It is more reliable for these labs, but is not the selected v0 path.

### Publish only from supported tags

`release.yml` should trigger on `push` tags matching `v*`, then validate that the tag matches the release policy before building. For `v0.1.0-rc.1`, publish as prerelease and not latest. The workflow must not create, move, or retag.

Rationale: tag ownership remains with the maintainer and the local mirror flow.

Alternative considered: manual `workflow_dispatch` with a tag input. This is useful later, but tag push is a clearer public release source of truth.

### Use GitHub CLI for release publishing

Use `gh release create --verify-tag` for publishing, with `GITHUB_TOKEN` scoped to the publish job. Generate or attach notes and upload all assets plus checksums and manifest.

Rationale: `gh` is available in GitHub Actions and avoids adding a release action dependency.

Alternative considered: third-party release actions. Rejected to keep the trust surface small.

### Build metadata remains minimal

If tagged release binaries do not report the tag through Go build info, add the smallest project-local build metadata hook needed for the release version to appear in LocalAPI/panel version output. Avoid broad version package refactors.

Rationale: release users need to identify the installed build, but this should not turn into an application versioning redesign.

Alternative considered: rely only on VCS revision. Rejected because release assets should report the human release tag when available.

## Risks / Trade-offs

- [Risk] GitHub-hosted core lab gates may be very slow or timeout without `/dev/kvm`. -> Mitigation: split lab workflows, cache the base image, upload artifacts/logs, and treat core lab timeout as release-blocking.
- [Risk] Scenario 1/2/3 gates may fail or timeout on GitHub-hosted runners even when they pass locally. -> Mitigation: keep `lab-scenarios.yml` as a manually runnable diagnostic workflow and require maintainers to run scenario gates locally before pushing a release tag.
- [Risk] Windows desktop installer build may require more Wails/NSIS setup than the current README documents. -> Mitigation: implement CI setup explicitly and keep Windows build sanity in `go-checks.yml` so failures block release early.
- [Risk] Packaging scripts may emit non-deterministic version strings. -> Mitigation: pass the release tag into package version metadata in CI instead of relying only on `0.0.0+git<sha>`.
- [Risk] Artifact fan-in across jobs can drift from release asset expectations. -> Mitigation: generate `release-manifest.json` from the final upload directory and verify every listed asset has a checksum before publishing.

## Migration Plan

1. Add OpenSpec artifacts and validate them.
2. Apply code changes in a later step:
   - fix Windows cross-build;
   - add minimal build metadata hook;
   - add workflows and any small packaging script switches needed for CI.
3. Run local host validation.
4. Push a non-release branch and manually run build/core lab workflows for diagnosis; use the scenario workflow only as best-effort hosted-runner diagnostics.
5. Run scenario 1/2/3 gates locally before tagging.
6. Push annotated tag `v0.1.0-rc.1` to the current `origin`; verify the mirror syncs to GitHub and the release workflow publishes only after host, build, and core lab gates pass.

Rollback is tag-level: delete the failed GitHub Release and tag from GitHub/current mirror only if the tag was published incorrectly. Do not move a public release tag silently; create a later RC tag for fixes.

## Open Questions

- None for the v0 plan. The selected release-blocking runner strategy is GitHub-hosted Ubuntu for core lab gates, with scenario gates handled as local pre-release validation.
