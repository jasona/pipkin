# Pipkin first-run experience — implementation contract

Owner-approved direction: self-contained local app, a warm Pipkin-branded welcome, connect a provider,
choose a model/project, then enter a useful conversation. Windows follows Mac evaluation. This extends
our existing desktop visual identity, not a rebrand. No completion-rate or setup-time claims exist yet.

## Direction contract

**THESIS:** first use feels like entering Pipkin, not administering a daemon. Keep engine work backstage;
expose actionable setup only when needed. No technical tour or ornamental success screen.

**OWN-WORLD:** inherit `docs/DESIGN.md` semantic surfaces, restrained accent, existing controls and bundled
fonts. Use the approved mascot as the welcoming anchor, not a collection of generic provider icon tiles.
Support both themes, large text, native keyboard focus and reduced motion.

**STORY:** “A little help for your next big thing.” Get started → Connect your AI → “What are we working on?”
→ conversation ready to write. Existing connections require an explicit reuse choice. Advanced users can
explore the workspace or explicitly choose an external engine without pretending setup is complete.

**FIRST VIEWPORT:** a quiet full-window surface, bounded reading column, generous breathing room, Pipkin
wordmark/mascot above the headline, brief supporting copy, one primary Get started action and a subordinate
Explore the workspace action. Consistent action placement on subsequent screens; back preserves choices.

**FORM:** owner-selected compact welcome/provider/project sequence in the established Operate world;
precise approved flow, not an open visual-concept tournament. No image-generation tool is available here.

**FINISH:** unreviewed and undocumented is unfinished; this build ends with the finish review, the verdict,
DESIGN.md, and every shipping raster carrying its provenance.

## State and honesty

- First run is an explicit setup disposition, not merely an empty conversation list. Returning users,
  saved work, demo mode and explicit external-server launches retain normal workspace/recovery behavior.
- Welcome never calls missing installation “Offline.” Distinguish preparing local engine, missing engine,
  provider choice, pending provider connection, failed connection, project choice and ready.
- Ready requires engine-confirmed readiness, acknowledged provider configuration, an available engine-selected
  model and a selected usable project. It does not imply a paid/model request succeeded. Model choices stay engine-authoritative.
- Existing credential metadata is a discovery result, not consent or a successful provider connection.
  Read credential metadata without publishing keys. No secrets in ordinary core state, persisted wizard
  progress, diagnostics, logs, crash annotations or error strings.
- Async setup results carry an attempt/epoch guard. A response from a previous retry cannot complete the
  current attempt. Interrupted setup resumes from acknowledged facts; no automatic prompt dispatch.
- Explore does not mark setup complete. Completion is durable only after acknowledged configuration and
  deliberate workspace entry. A returning user's provider failure must never restart the welcome tour.
- Do not replace the user's app/engine profile while designing or testing. Use disposable profiles/projects.

## Engine contract findings and required implementation work

The current Mac `.app` does not bundle Pi/Node. Existing managed launch assumes a system Node PATH; Finder
startup cannot rely on Terminal's shell configuration. Ship a pinned, verified architecture-matched runtime
and engine with retained notices, then qualify that actual pair in a bare PATH/disposable profile. Do not
mutate an owner's global Node installation or download executable updates without an explicit setup policy.

The pinned experimental Pi service currently exposes model/status/refresh operations, **not a provider
login or API-key-write RPC**. `auth-storage.ts` says auth orchestration belongs to ModelRuntime/pi-ai Models.
A beautiful credential screen cannot paper over that missing contract. Implement/test an owned-engine auth
boundary, backed by Pi's credential mechanism, before enabling connection actions. OAuth methods differ by
provider; no universal “sign in with your subscription” promise. Never collect a key in a normal composer.

## Delivery sequence

1. Tested first-run policy and attempt-scoped setup state (foundation; not a shipped screen).
2. Self-contained Mac engine/runtime discovery, lifecycle and notice/signature qualification.
3. Typed provider capability/auth boundary, secret-safe storage/error handling and synthetic auth tests.
4. Full welcome/provider/project surface using real acknowledgements, persisted progress and existing controls.
5. Bounded native visual review (light/dark, constrained viewport/large text), owner Mac onboarding through a
   real first reply, interruption/failure review, then revise. Headless tests are not native acceptance.

The original first-run engine-directory message and the unnotarized Mac download remain separate findings.
Ad-hoc signing fixes bundle integrity, not normal Gatekeeper trust; Developer ID/notarization is still needed.

## Current delivery status

The design/flow direction is approved. The foundational policy can be unit-tested independently of GPUI.
No implemented credential UI, bundled runtime, setup completion observation or native visual review is claimed
by this document. Product/design context contains prototype-era statements; use current source and release-gate
evidence for capabilities rather than silently repairing unrelated context as part of this feature.
