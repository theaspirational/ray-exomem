# UI Polish Design Spec

## Goal

Polish the ray-exomem Svelte UI: consolidate standalone routes into ExomView tabs, establish a shared component design system, fix theme compliance, add overflow handling for narrow screens, and achieve visual consistency across all views.

## Architecture

Two parallel tracks, each producing independent working commits:

- **Track A — Design System Foundation**: shared components, spacing conventions, theme fixes. No feature changes.
- **Track B — Route Consolidation + Visual Consistency**: extract standalone route logic into reusable components, wire into ExomView tabs, delete standalone route wrappers, apply shared components everywhere.

Track A lands first. Track B consumes Track A components.

## Tech Stack

- Svelte 5 (runes), shadcn-svelte (bits-ui), Tailwind 4, @lucide/svelte
- D3 (existing, for graph), marked (existing, for guide)
- No new dependencies

---

## 1. Shared Components

Five reusable primitives in `ui/src/lib/components/`:

### EmptyState

Centered message with optional icon and action button.

```
Props:
  icon?: Lucide component (default: Package)
  message: string
  actionLabel?: string
  onAction?: () => void
```

Replaces 6+ instances of `<p class="text-sm text-zinc-500">No X</p>` across ExomView, FolderView, SessionFactsPanel.

### ErrorState

Red-bordered error box with retry button.

```
Props:
  message: string
  onRetry?: () => void
```

Replaces 3 identical error blocks in ExomView (facts, branches) and future tab content.

### LoadingState

Spinner with message.

```
Props:
  message?: string (default: "Loading…")
```

Replaces repeated `<Loader2 class="size-4 animate-spin" />` + text patterns.

### StatCard

Label/value pair in a bordered card.

```
Props:
  label: string
  value: string | number
  icon?: Lucide component
```

Used in ExomView header (facts count, branch name, branch count) and FolderView summaries.

### DataRow

Structured list item with icon, label, badges, and trailing info.

```
Props:
  icon?: Lucide component
  label: string
  badges?: Array<{ text: string, variant: 'default' | 'secondary' | 'outline' }>
  trailing?: string | Snippet
  onclick?: () => void
```

Used in branches list, and anywhere a list of named items with metadata appears.

### Placement

All five components go in `ui/src/lib/components/` (not under `ui/` — these are app-level, not shadcn primitives).

---

## 2. Spacing Conventions

Not CSS custom properties — Tailwind utility classes applied consistently via shared component defaults:

| Token | Classes | Usage |
|-------|---------|-------|
| page-x | `px-4 sm:px-6` | Horizontal page/section padding |
| page-y | `py-4 sm:py-6` | Vertical page/section padding |
| section-gap | `gap-4` | Between major sections in a view |
| item-gap | `gap-2` | Between list items |
| inline-gap | `gap-2` | Between inline elements (icon+text, badges) |

Applied by convention when building/refactoring views. No config layer.

---

## 3. Theme Fixes

### Graph Colors (applied during GraphPanel extraction)

Current (`ui/src/routes/graph/+page.svelte`): 15 hardcoded hex colors in `PALETTE` array + inline D3 styles with hex values.

Fix (applied when creating `GraphPanel.svelte` in Track B):
- Extend `app.css` chart palette from 5 to 10 colors (add `--chart-6` through `--chart-10`) — this part lands in Track A
- Replace `PALETTE` array with `getComputedStyle()` reads of `--chart-N` CSS vars
- Replace inline D3 hex colors (`#f59e0b`, `#1e293b`, `#94a3b8`, `#e2e8f0`) with CSS var equivalents

### Guide Page (`ui/src/routes/guide/+page.svelte`)

Current: `<style>` block with 12 hardcoded `rgb()` values.

Fix:
- Replace all `rgb()` values with CSS var references (`var(--foreground)`, `var(--border)`, `var(--muted-foreground)`, etc.)
- Use Tailwind `@apply` where appropriate for text/background colors

---

## 4. Route Consolidation

### Routes to Extract → Wire into ExomView Tabs

| Standalone Route | Component Extracted | ExomView Tab | Notes |
|---|---|---|---|
| `/facts` (776 lines) | `FactsManager.svelte` | Facts | Full facts management: filter, edit, delete, assert, bulk import/export. Replaces current simple `FactsDataTable` in Facts tab. |
| `/graph` (884 lines) | `GraphPanel.svelte` | Graph | D3 force-directed visualization + controls. Receives `exomPath` prop. |
| `/rules` (647 lines) | `RulesPanel.svelte` | Rules | Add/edit/delete inference rules with syntax highlighting. |
| `/timeline` (195 lines) | `TimelinePanel.svelte` | History | Fact timeline with filtering. Replaces "Coming soon" in History tab. |
| `/provenance` (640 lines) | `ProvenancePanel.svelte` | — | Reachable as drill-down from fact rows in FactsManager (click fact → see lineage). Not a tab. |

### Routes Kept Global

| Route | Reason |
|---|---|
| `/query` (359 lines) | Power-user tool, benefits from full-page layout. Added to CommandPalette actions. |
| `/guide` (133 lines) | Reference doc, already linked. Theme fixes only. |

### Route Files Deleted After Extraction

- `ui/src/routes/facts/+page.svelte`
- `ui/src/routes/facts/[id]/+page.svelte` (logic moves into ProvenancePanel or fact detail within FactsManager)
- `ui/src/routes/graph/+page.svelte`
- `ui/src/routes/rules/+page.svelte`
- `ui/src/routes/timeline/+page.svelte`
- `ui/src/routes/provenance/+page.svelte`

### Extracted Component Location

All extracted components go in `ui/src/routes/tree/[...path]/` alongside existing ExomView, FolderView, etc. They are view-level components specific to the tree route, not shared library components.

---

## 5. Drawer Changes

### Search Button

Currently: placeholder text "Search placeholder — Phase 8 fills this".

Fix: Search button in drawer rail calls `CommandPalette.open()`. Share open state via an exported function or svelte store. No new search UI.

### Settings Panel

Currently: placeholder text "Settings placeholder — Phase 8 fills this".

Fix: Remove placeholder entirely. No settings panel for now.

### CommandPalette Actions

Add "Open Query Editor" action to CommandPalette that navigates to `/query`.

---

## 6. Responsive & Overflow

Strategy: graceful degradation. Overflow handling only, no layout restructuring.

### TopBar Breadcrumb

- Wide screens: full path with all segments as clickable links. Branch badge inline at end (flex row, not absolute positioned).
- Narrow screens: show first segment + `…` button + last segment. Click `…` → dropdown with all path segments, each clickable/navigable. Uses Popover or DropdownMenu from shadcn.
- Collapse threshold: when breadcrumb container overflows (ResizeObserver or segment count > 3).

### Per-Component Overflow Fixes

| Component | Fix |
|---|---|
| FactsDataTable / FactsManager | `overflow-x-auto` wrapper around table |
| GraphPanel | SVG gets `min-width: 600px` inside `overflow-x-auto` container. Controls panel stacks below via `flex-col` at narrow width. |
| FolderView grid | Already responsive (`sm:grid-cols-2 lg:grid-cols-3`). No change. |
| CommandPalette | Already uses viewport-relative sizing. No change. |
| SessionFactsPanel kanban | `overflow-x-auto` on kanban columns container |
| RulesPanel | `overflow-x-auto` on code blocks |

### Explicit Non-Goals

- No mobile-specific layouts (no bottom nav, no card-based tables)
- No touch gesture handling (graph stays mouse-only)
- No drawer collapse to hamburger menu
- No viewport-specific component swaps

---

## 7. Visual Consistency

### ExomView Header

Current: path + inline "Facts N" text + badges crammed together.

After: path + kind badge on first line. Row of StatCard components below (Facts count, current branch with icon, branch count). Clean visual hierarchy.

### Branches List

Current: inline HTML `<li>` elements with mixed flex layouts.

After: DataRow components with consistent icon (`GitBranch`), label, badges (`current`, `archived`), trailing info (fact count, claimed_by).

### FolderView Children Cards

Current: Card with Lucide Folder/Box icons + redundant "folder"/"exom" Badge below.

After: Drop the kind Badge (icon already conveys type). Use `Brain` icon (from `@lucide/svelte`) for exoms instead of `Box`. Show stats inline below name ("42 facts · ⎇ main" for exoms, "3 children" for folders).

### Empty/Error/Loading States

All views switch from ad-hoc inline markup to EmptyState, ErrorState, LoadingState shared components. Consistent look across every tab and view.

---

## 8. File Changes Summary

### New Files

- `ui/src/lib/components/EmptyState.svelte`
- `ui/src/lib/components/ErrorState.svelte`
- `ui/src/lib/components/LoadingState.svelte`
- `ui/src/lib/components/StatCard.svelte`
- `ui/src/lib/components/DataRow.svelte`
- `ui/src/routes/tree/[...path]/FactsManager.svelte` (extracted from `/facts`)
- `ui/src/routes/tree/[...path]/GraphPanel.svelte` (extracted from `/graph`)
- `ui/src/routes/tree/[...path]/RulesPanel.svelte` (extracted from `/rules`)
- `ui/src/routes/tree/[...path]/TimelinePanel.svelte` (extracted from `/timeline`)
- `ui/src/routes/tree/[...path]/ProvenancePanel.svelte` (extracted from `/provenance`)

### Modified Files

- `ui/src/app.css` — extend chart palette to 10 colors
- `ui/src/lib/Drawer.svelte` — search button triggers CommandPalette, remove settings placeholder
- `ui/src/lib/TopBar.svelte` — breadcrumb overflow dropdown, branch badge inline flex
- `ui/src/lib/CommandPalette.svelte` — add "Open Query Editor" action, export open function
- `ui/src/routes/tree/[...path]/ExomView.svelte` — wire all 5 tabs, StatCard header, shared components
- `ui/src/routes/tree/[...path]/FolderView.svelte` — Brain icon, drop kind badge, stats inline
- `ui/src/routes/tree/[...path]/FactsDataTable.svelte` — overflow-x-auto wrapper
- `ui/src/routes/tree/[...path]/SessionFactsPanel.svelte` — kanban overflow, shared components
- `ui/src/routes/guide/+page.svelte` — CSS vars instead of hardcoded rgb
- `ui/src/routes/+layout.svelte` — minor adjustments for CommandPalette open state sharing

### Deleted Files

- `ui/src/routes/facts/+page.svelte`
- `ui/src/routes/facts/[id]/+page.svelte`
- `ui/src/routes/graph/+page.svelte`
- `ui/src/routes/rules/+page.svelte`
- `ui/src/routes/timeline/+page.svelte`
- `ui/src/routes/provenance/+page.svelte`

---

## 9. Testing Strategy

- `npm run check` — Svelte type checking after each component change
- `npm run build` — verify static build succeeds (embedded in daemon)
- Visual verification via `ray-exomem serve` + browser: check each ExomView tab, FolderView, TopBar at various widths
- Verify CommandPalette "Open Query Editor" action navigates correctly
- Verify breadcrumb overflow dropdown appears and segments are clickable
- `cargo test` — ensure Rust integration tests still pass (binary embeds UI build)
