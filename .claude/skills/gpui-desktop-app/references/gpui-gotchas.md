# GPUI gotchas (verified in this build or in Zed's source at rev a846890)

Paths: G = `zed/crates/gpui/src`, `.rules` = `zed/.rules` (Zed's own rules, also CLAUDE.md/AGENTS.md there).

## Entities, tasks, subscriptions
- Dropping a `Task` cancels it (`#[must_use]`). Hold it in a field, `.await` it, `.detach()` or `.detach_and_log_err(cx)`. `cx.spawn(async move |this, cx| ..)` gives `this: WeakEntity<T>` and `&mut AsyncApp`; `this.update(cx, ..)` returns `Result` (entity may be gone). `cx.background_spawn` for `Send` work on other threads; UI work stays on the one foreground thread. (.rules "Concurrency"; G/executor.rs)
- `cx.subscribe/observe/on_release` return `#[must_use] Subscription` that deregisters on drop. Keep them in `_subs: Vec<Subscription>` (we did). `subscribe_in/observe_in` give you `&mut Window`.
- Inside `entity.update(cx, |this, cx| ..)` use the closure's `cx`, never the outer one. Updating an entity that is already being updated panics ("cannot update X while it is already being updated", G/app/entity_map.rs:231). Escape hatch: `cx.defer(..)` (runs at end of the effect cycle). Zed uses it from a list scroll handler because `ListState` is mutably borrowed during the callback (agent_ui thread_view.rs ~1063).
- Avoid mutually recursive strong handles; use `WeakEntity` for back-references.
- In GPUI tests use `cx.background_executor().timer(d).await`, not `smol::Timer`, or `run_until_parked` misses it (.rules).
- This prototype's `Model::finish` takes the effect handler out of an `Option`, calls it per effect, and puts it back, so handler code that re-enters `Model` doesn't hit a borrow problem.

## Rendering
- `Render` (`&mut self` + `Context<Self>`) is for stateful views; `RenderOnce` (by value, `&mut App`) for stateless components (our `Btn`). `cx.notify()` when render-affecting state changes. Notify is deduplicated; a notify during draw does not schedule a frame (G/window.rs:176).
- Uncached child entities re-run `render` every frame the parent draws. `Entity::cached(style)` / `AnyView::cached` reuse the previous subtree unless that entity notified, the window refreshed, or bounds/mask/text-style changed; it needs a definite size (G/view.rs:25-306). Zed uses it only for big panels (dock.rs, pane_group.rs).
- Element ids must be unique per frame. Two views keyed on the same entity must not be siblings (state collides). `use_keyed_state` for list items (G/window.rs:4204); `use_state` keys by code location. `with_element_state` panics on reentrancy.
- `text!` ids come from source location: in loops use `.with_id(i)` or wrap in `div().id(i)` (G/_accessibility.rs).
- `Window::refresh()` bypasses caches; `request_animation_frame` re-renders the view; decorative motion should use `with_animation` (respects `reduce_motion`). Frames are driven by the compositor frame callback and only draw when dirty (G/window.rs:1709). Streaming should invalidate only the streaming item.

## Elements and styling
- **Interactive/a11y methods live on `Stateful<Div>`**: `div().id("x").role(Role::Button).aria_label(..)`. Without `.id()` they don't exist. Role types: `gpui::Role`.
- `focus_visible(|s| ..)` shows only after keyboard input (G/window.rs InputModality); needs `track_focus`/tab stop. `tab_stop(true)` makes a focusable stop; `tab_index(n)` implies it; `tab_group()` scopes indices.
- Menus/overlays: `deferred(child).with_priority(n)` paints above ancestors but keeps layout; add a full-window backdrop with `.occlude()` and `on_click` for dismissal. `anchored()` children must have no margin. We placed menus with absolute positioning inside a deferred root overlay.
- Key dispatch: `.key_context("Name")` + `.on_action(cx.listener(..))` + `track_focus`; bindings are context-scoped. Put `key_context("Overlay")` only on overlays that don't contain a text input, or its Up/Down/Enter bindings shadow the editor's. Actions only reach the focus path.
- Drag-resize splitters: `.on_drag(Marker, |_,_,_,cx| cx.new(|_| Ghost))` on the divider, `.on_drag_move::<Marker>(..)` on the root, keep a **local live width** during the drag and commit one `SetWidth` command on mouse-up (otherwise you write prefs on every move).
- `uniform_list` inside a `flex_1().min_h_0()` wrapper rendered **zero rows** (blank diff). Give the list `.size_full()` inside a sized parent. Horizontal scroll: `.with_horizontal_sizing_behavior(ListHorizontalSizingBehavior::Unconstrained)` + `.with_width_from_item(Some(index_of_widest))`; row divs need `min_w_full`.
- `svg().path("icons/x.svg").text_color(c)` colors Lucide icons (stroke=currentColor). Provide an `AssetSource` (`Application::with_assets`) and `cx.text_system().add_fonts(vec![Cow::Borrowed(include_bytes!(..))])`. Font families by name ("IBM Plex Sans", "Lilex").
- `window.viewport_size()` in render drives responsive docking; rerenders on resize automatically.
- `cx.prompt_for_paths(PathPromptOptions{..})` returns a oneshot of `Result<Option<Vec<PathBuf>>>`: outer `Err` = picker unavailable (show a toast), `None` = cancelled. Do file metadata in `cx.background_spawn`.
- `Application::with_assets(..)` is on the builder returned by `gpui_platform::application()`. `app_id` and `titlebar.title` in `WindowOptions`; `window.set_window_title` from an observer.

## Accessibility (AccessKit via AT-SPI on Linux)
- `gpui_platform::application()` has a11y on by default (Zed itself gates on `ZED_EXPERIMENTAL_A11Y=1`). Only elements with a `GlobalElementId` **and** a role become nodes; duplicate ids are silently dropped in release.
- Real tree from this app (python `Atspi`): landmarks Navigation/Conversation/Changes inspector; `log` "Conversation transcript" with `article` per message; `status bar` "Run status: …"; entries; buttons; `document frame` for the diff. Headings had empty names until given `aria_label`; the nav search entry was named "Message" until `set_label`. Always dump the tree.
- Not exposed: caret/selection as AT-SPI text, per-run text nodes, disabled flag. Only mounted rows appear.

## Misc
- Add `env_logger` (or any `log` backend) or you will not see GPUI's adapter-selection logs.
- `gpui` dev-dep with `features = ["test-support"]` for `TestAppContext`; test-support turns on wayland+x11+proptest. Leak detection is separate (`GPUI_LEAK_DETECTION` env at build time or `leak-detection` feature); `cargo test` inside the gpui crate always has it.
- Zed bans `unwrap()` and `let _ =` on fallible calls; use `?`/`log_err()`.
