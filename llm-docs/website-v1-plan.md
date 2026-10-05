# Pipkin website — version 1 plan

## 1. Purpose

Build a small, product-led website that introduces Pipkin, shows the desktop interface, and helps interested developers follow the project or try its development build.

**Recommended positioning:** a native desktop workspace for Pi’s coding-agent workflow, built with Rust and GPUI.

The website should answer four questions quickly:

1. What is Pipkin, and how does it relate to Pi?
2. What does using it look like?
3. How usable is it today, and on which platforms?
4. Where can I follow development or try it?

This is a launchpad for an early project, not a mature-product sales funnel. Trust comes from showing the actual application and being precise about readiness.

### Planning assumptions

- Primary audience: individual developers already interested in Pi, native developer tools, or Linux desktop software.
- Secondary audience: contributors evaluating the architecture and project direction.
- Primary conversion: visit the source repository. Secondary conversion: read the development setup instructions.
- English-only, static site, no accounts, payments, or mailing-list backend for v1.
- Repository URL, domain, public license, and distribution strategy must be confirmed before launch.
- The proposed composition and copy below are recommendations, not an approved website identity.

## 2. Product truth and claim boundaries

Use `docs/rust-desktop-client-plan.md`, especially its **Implementation status**, as the primary readiness reference. `README.md` supplies running instructions; `docs/DESIGN.md` and `docs/captures/` supply visual evidence. The prototype scorecard documents narrower, historical verification.

There is documentation drift: `docs/PRODUCT.md` still describes real integration as future work, while the newer implementation status records M2 completion. The README and older M1 notes also describe a read-only real client. Resolve public setup instructions against the actual launch build rather than combining these accounts into an unsupported promise.

| Topic | Safe v1 treatment |
| --- | --- |
| Identity | Native Rust/GPUI desktop client for Pi; not a separate coding model or a replacement agent engine. |
| Current maturity | In development. The roadmap records a first real workflow validated by real-engine tests using a deterministic test provider. The complete M2 workflow has not been verified in the GUI. |
| Platform | Current validation target: Omarchy/Hyprland on Wayland. Other Linux environments, macOS, and Windows remain unqualified. |
| Interface | Show conversation navigation, a multiline composer, transcript, tool activity, and changes inspector as visible UI—not as proof that every real-backend action is ready. |
| Integration | Explain that real mode requires the experimental Pi service and development setup, including Pi-side changes. It is not yet a packaged consumer installation. |
| Demo | Retain **“Demo · simulated agent”** in screenshots and recordings. Add a nearby caption explaining that execution is simulated. |
| Upcoming workflows | Real steering, queueing, attachments, and history paging belong in a clearly labelled roadmap, not the shipped-features list. |
| Reliability | Describe recovery as engineering work and tested behavior with explicit scope; do not promise exactly-once tool execution or zero data loss. |
| Performance | Omit numeric marketing claims in v1. Existing figures are prototype measurements, not integrated-app benchmarks or display-presentation latency. |
| Accessibility | Commit to an accessible website; do not imply the native app has passed screen-reader, IME, or display-scale qualification. |

Avoid “production-ready,” “works everywhere,” “fully accessible,” “instant,” and “private by default” unless supported by specific evidence. Local desktop software does not mean model requests stay on the device.

## 3. Scope and information architecture

### Ship in v1

**`/` — product overview**

A single coherent page with the interface as the central evidence, a short workflow story, honest status, and links to the repository and getting-started guide.

**`/getting-started/` — development preview guide**

A compact guide covering supported/tested environment, prerequisites, demo launch, real-mode requirements, and troubleshooting links. Clearly separate simulated and real execution.

**`/404.html` — useful not-found page**

Offer links to the homepage, getting-started guide, and repository.

Keep roadmap and FAQ on the homepage initially. Link technical documentation to the repository instead of duplicating the entire documentation tree.

### Explicitly out of scope

- Pricing, team plans, accounts, hosted workspaces, or billing.
- Download buttons without tested release artifacts.
- Waitlist forms without an agreed delivery and privacy plan.
- Blog, documentation search, CMS, community forum, and localization.
- Browser-based reimplementation of Pipkin or a fake interactive agent session.
- Testimonials, customer logos, comparison tables, or unsupported benchmark charts.

## 4. Homepage content and sequence

### A. Header

- Pipkin wordmark; use a simple text treatment until a logo is approved.
- Links: **Interface**, **Project status**, **Getting started**, **GitHub**.
- No elaborate navigation menu. On narrow screens, allow a compact wrapping header rather than adding a hamburger for four links.
- All destinations must be real; confirm the repository host before using “GitHub.”

### B. Opening: product and artifact together

**Proposed headline:**

> A native desktop workspace for Pi.

**Proposed supporting copy:**

> Follow conversations, read tool activity, and inspect workspace changes in a focused Rust/GPUI interface.

**Primary action:** `View source`.

**Secondary action:** `Try the development demo` → the demo section of getting started.

**Visible status line:**

> In development · Targeting Omarchy / Wayland first

Place a large, real desktop capture immediately beneath or alongside the introduction. Keep surrounding website chrome sparse. The first viewport should make clear that this is an actual desktop application, not another chat website.

Caption the capture:

> Pipkin’s desktop interface. Demo · simulated agent. Real Pi integration is in development.

Do not hide the maturity warning in the footer or remove the simulation marker to make the image look more finished.

### C. A close look at the workspace

Use one substantial annotated screenshot rather than a grid of generic feature cards. Three numbered captions explain:

1. **Keep the conversation readable.** Prompts, responses, and expandable tool activity have distinct visual roles.
2. **Stay with the work.** The transcript provides selection and a way back to the latest activity while reading older output.
3. **Inspect changes beside the conversation.** The changes pane keeps file differences visible without presenting Pipkin as a full editor.

Use DOM text for captions and only nonessential visual markers over the image. Link to a full-size capture. On mobile, pair each caption with a readable crop instead of shrinking the entire desktop workspace to illegibility.

### D. Explain the relationship to Pi

Heading: **Pi is the agent. Pipkin is the desktop workspace.**

Short paragraph explaining that Pi supplies agent execution and authoritative session state, while Pipkin provides the native interface and desktop-owned state such as drafts and preferences.

A simple text sequence can communicate the intended workflow:

> Open a project → start or resume a conversation → follow the work → inspect changes → continue or stop

Label this as the **product direction**, not a promise that every step is currently complete. Link to Pi’s confirmed public project URL and Pipkin’s architecture document.

### E. Project status

Use a compact status table with explicit labels:

| Area | Public-facing summary |
| --- | --- |
| Native interface | Implemented prototype; selected interactions observed on the target Wayland machine. |
| Real Pi integration | First real workflow validated in engine tests; full GUI validation and packaging remain ahead. |
| Daily-use readiness | In progress; execution/recovery work and platform qualification remain release gates. |
| Broader platforms | Not yet qualified. |

Include `Last reviewed: YYYY-MM-DD`, populated when the website is actually reviewed. Link to the detailed roadmap and verification scorecard. Do not present progress percentages or dates for uncommitted releases.

### F. Short FAQ

Answer these questions in plain language:

- **Is Pipkin a new agent?** No; it is a desktop client for Pi.
- **Can I use it today?** Developers can try the demo or follow the experimental real-mode setup; it is not yet a packaged daily-use release.
- **Which platforms are supported?** State the tested target and distinguish future qualification from availability.
- **Does everything run locally?** The interface and local engine are desktop-oriented; model-provider requests depend on Pi configuration.
- **Does it replace my editor or terminal?** No; the direction is a focused agent workspace, not a complete IDE.
- **How can I help?** Read the repository’s contribution instructions, report reproducible issues, or follow development. Do not imply an established contribution policy if none exists.

### G. Closing action and footer

Close with **Follow Pipkin’s development** and the same `View source` primary action. Keep `Getting started` secondary.

Footer: repository, roadmap, verified license link, and a brief status note. No decorative partner badges, fabricated social proof, or empty social accounts.

## 5. Visual direction

**Recommendation: an interface-led product page that extends the native app’s identity.** The memorable object is the real Pipkin workspace, not an abstract AI illustration.

- Inherit the app’s neutral layered surfaces, restrained blue accent, subtle dividers, and precise spacing.
- Prefer a light reading surface with a dark app capture as the focal object. This provides contrast without making the entire page a glowing developer-tool template. Confirm this choice during website design.
- Reuse IBM Plex Sans for continuity with the actual application, not as generic tech branding. Use Lilex sparingly for commands and shortcut labels. Verify web embedding licenses and ship only required font files/subsets.
- Give the headline generous scale and use short, direct text. Keep long-form guide text within a comfortable reading measure.
- Alternate an expansive product image with quieter explanatory passages. Avoid repeating identical cards down the page.
- Preserve screenshots’ proportions and legibility; no perspective distortions, artificial window chrome, or invented UI.
- Avoid gradient blobs, neon glows, stock developer photos, continuous typing animations, and a “powered by AI” visual vocabulary.
- Motion is optional and subordinate. No autoplay video, scroll hijacking, or content hidden behind entrance animation.

A screenshot reveal or click-to-enlarge interaction is sufficient for v1. Do not spend the first release budget on a simulated live terminal.

## 6. Assets and content preparation

### Existing material

Candidate captures in `docs/captures/`:

- `dark-streaming-diff.png`: primary workspace candidate.
- `light-diff.png`: theme detail or secondary capture.
- `palette.png`: keyboard-command detail if space permits.
- `scrolled-away-jump-to-latest.png`: transcript behavior detail.
- `empty-state.png`: potential getting-started illustration.

These are prototype evidence, not automatically launch-ready images. Review each for freshness, readability, private paths, background bleed from translucent windows, and whether its content is simulated. Do not infer current integration readiness from an older capture.

### Required before implementation

1. Choose one hero capture and two or three useful detail crops.
2. Capture replacements if existing images are stale or show other windows through transparency. Any native capture work must follow the repository’s testing guard rules.
3. Preserve demo indicators and write honest captions and meaningful alt text.
4. Confirm public repository/Pi URLs, domain, license, and public issue/contribution destinations.
5. Validate getting-started commands against the intended public revision.
6. Export optimized responsive images with a record of source capture and demo/real status.

A short, user-initiated screen recording can be added later. It is not a launch dependency.

## 7. Getting-started guide

Structure the page as follows:

1. **Before you start:** development-build warning and current platform target.
2. **Prerequisites:** tested Rust toolchain and system requirements from the repository, with links rather than guessed package-install commands.
3. **Run the simulated demo:** prominently labelled `Demo · simulated agent`; provide the verified command from the README.
4. **Connect to real Pi:** explain experimental service requirements and link to the maintained setup instructions. Do not offer a one-line installation that skips Pi-side patches or version constraints.
5. **What to expect:** separate implemented UI, tested real integration, and incomplete daily-use features.
6. **Troubleshooting and feedback:** repository documentation and issue link; remind users to redact credentials and sensitive project information.

Use readable code blocks with an optional copy button that reports success/failure. Keep commands selectable without JavaScript. Do not publish commands for downloading binaries until release artifacts exist and are tested.

## 8. Technical approach

**Recommended stack:** Astro with static output, ordinary CSS, and Markdown content. A small static HTML implementation is equally acceptable if preferred; no application framework is needed for this scope.

- Keep website sources independent of the Rust crates, for example in `website/`.
- Build the two content routes and a real 404 page.
- Use semantic HTML and little or no client JavaScript. Add JavaScript only for progressive enhancements such as copy feedback or accessible image enlargement.
- Host on a static service chosen after the domain and repository host are confirmed.
- CI: clean install, static build, link checks, and available accessibility checks; deploy previews before production.
- Do not introduce cookies, analytics, external font requests, or marketing scripts by default.
- Add page titles, descriptions, canonical URLs once the domain is known, Open Graph imagery, favicon, sitemap, and robots file.
- Set explicit image dimensions, responsive sources, and modern compressed formats. Load the hero image eagerly; lazy-load below-the-fold images.
- Make the content usable with JavaScript disabled.

## 9. Accessibility, responsive behavior, and performance

These are website acceptance requirements, not claims about native-app qualification.

- Aim for WCAG 2.2 AA: contrast, meaningful headings, skip link, visible keyboard focus, labelled controls, and adequate target sizes.
- Make navigation, code copying, FAQ controls, and any image viewer keyboard-operable. An enlarged viewer must close with Escape and restore focus.
- Respect reduced motion. Never make information depend on hover, color alone, or animation.
- Verify 320–390 px mobile widths, tablet, and common desktop widths, plus 200% zoom and long text. Only code or intentionally full-size images may scroll horizontally within their own containers.
- On mobile, place copy and actions first, then a readable product preview and detail crops. Do not redesign the desktop application into a fictional mobile app.
- Target minimal client JavaScript, no layout shifts from images/fonts, and an initial page transfer around 1 MB or less through image optimization.
- Treat Lighthouse scores and Core Web Vitals thresholds as targets until measured. Record lab results with device/network conditions; do not claim real-user performance without field data.

## 10. Implementation sequence

### Phase 1 — Verify the public story

- Resolve the README/roadmap status discrepancy for the intended release revision.
- Confirm links, supported environment wording, and primary CTA.
- Approve the proposed copy and select truthful screenshots.

**Exit:** publishable facts, destinations, and asset list are agreed.

### Phase 2 — Design the two pages

- Produce desktop and mobile compositions for the homepage.
- Set website tokens derived from, but not replacing, the native app design record.
- Establish the getting-started reading layout and screenshot/caption treatment.

**Exit:** a visitor can identify the product, maturity, and next action without needing the FAQ.

### Phase 3 — Implement the static site

- Build routes, shared header/footer, responsive images, guide content, metadata, and 404 page.
- Add only necessary progressive enhancements.
- Configure preview deployment and build/link checks.

**Exit:** complete preview with no placeholders or invented claims.

### Phase 4 — Verify and launch

- Inspect desktop/mobile together, keyboard navigation, zoom, reduced motion, and image enlargement if present.
- Check every external link and every published command against the intended build.
- Audit captions for demo honesty and platform/readiness wording.
- Measure performance and accessibility; batch fixes and confirm once more.
- Configure domain, TLS, redirects, and production deployment; verify the live site.

**Exit:** launch checklist passes and any remaining limitations are documented.

## 11. Launch checklist

- [ ] First viewport says “native desktop workspace for Pi,” shows the app, and exposes a clear action.
- [ ] Development status is visible before a visitor commits to trying it.
- [ ] Demo assets retain the simulation marker and clear captions.
- [ ] No unsupported platform, accessibility, privacy, reliability, or performance claims.
- [ ] Source, Pi, roadmap, license, and setup links resolve publicly.
- [ ] Instructions match the intended public code revision and experimental engine requirements.
- [ ] Mobile screenshots remain understandable; no accidental page overflow.
- [ ] Keyboard, focus, contrast, zoom, and reduced-motion checks are recorded.
- [ ] Metadata, social preview, favicon, sitemap, and 404 are complete.
- [ ] Static build and link checks pass; live deployment is verified.
- [ ] Ownership is assigned for reviewing website status after each application milestone.

## 12. After v1

Add downloads only when tested packages and a supported compatibility contract exist. Add a real-mode recording after the complete workflow is observed in the GUI. Expand documentation when repeated setup questions justify it. Add release notes when there are public releases to explain.

The success criterion for v1 is not appearing bigger than the project is. It is making Pipkin understandable, tangible, and worth following—with a clear distinction between the interface already built, the integration verified so far, and the product still being completed.
