# miopunch

miopunch is being redesigned as an identity-based private network for personal
and small-group use. The September 2026 work is product and architecture
discussion, not an implementation release. There is no active runtime in the
repository root yet.

Start with [CONTEXT.md](CONTEXT.md), the [product direction draft](docs/notes/2026-09-27-product-mainline-draft.md), and the [scenario walkthrough](docs/notes/2026-09-28-network-scenario-walkthrough.md). These documents distinguish confirmed behavior from open design questions. Research reports in `docs/notes/` are evidence, not approved implementation plans.

The current [implementation specification](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/4700bf29-2a4e-40bc-93fe-7f5148207189)
collects the accepted decisions, module responsibilities, behavioral acceptance
criteria, and technical prerequisites still requiring validation. Use it as the
starting point for implementation planning; the runtime has not been built yet.

The former Go/Android prototype, OpenSpec artifacts, lab, scripts, and local
agent prompts are retained in [the legacy archive](archive/legacy-go-openspec-2026-09-28/README.md).
The archived implementation and test commands are not automatically the new
baseline. Whether to reuse, adapt, or replace the old validation tooling is
still undecided. Existing local build output in `dist/` is retained but not
tracked.

Work items and decision tickets live in the Plane `MIOPUNCH` project. See
[the agent tracker guide](docs/agents/issue-tracker.md).
