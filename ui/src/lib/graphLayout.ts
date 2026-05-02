// Shared utilities for deterministic + persisted force-graph layouts.
//
// Both the per-exom GraphPanel and the global /graph view run independent
// `d3.forceSimulation` instances. To make their output deterministic and to
// persist user-dragged positions across reloads/devices we:
//
// 1. Sort nodes by id and seed the simulation's RNG by `scope` so initial
//    placement is stable for a given input.
// 2. Store positions normalized to the saved layout's bounding box, so they
//    are independent of the canvas size and can be exported/shared.

export const GRAPH_LAYOUT_VERSION = 2;

/** d3-zoom transform: pan (x, y) and scale (k). */
export type ViewportState = { x: number; y: number; k: number };

/**
 * Force-graph control values. Exact set varies by view (the global graph
 * doesn't expose a controls panel today), so every field is optional.
 */
export type ControlsState = {
	linkDistance?: number;
	chargeStrength?: number;
	collisionRadius?: number;
	nodeScale?: number;
	labelSize?: number;
	edgeLabelSize?: number;
	linkWidth?: number;
	showNodeLabels?: boolean;
	showEdgeLabels?: boolean;
	showControls?: boolean;
};

export type LayoutPayload = {
	version: number;
	nodes: Record<string, [number, number]>;
	viewport?: ViewportState;
	controls?: ControlsState;
};

type Sized = { width: number; height: number };
type PositionedNode = { id: string; x?: number; y?: number };

// Small, stable 32-bit string hash (FNV-1a variant). Same input → same output
// across browsers and runs.
export function hashStr(s: string): number {
	let h = 0x811c9dc5;
	for (let i = 0; i < s.length; i++) {
		h ^= s.charCodeAt(i);
		h = Math.imul(h, 0x01000193);
	}
	return h >>> 0;
}

// Mulberry32 PRNG seeded from a string. d3-force accepts any `() => number`
// returning values in `[0, 1)` via `simulation.randomSource(...)`.
export function seededRng(scope: string): () => number {
	let a = (hashStr(scope) || 1) >>> 0;
	return function () {
		a = (a + 0x6d2b79f5) >>> 0;
		let t = a;
		t = Math.imul(t ^ (t >>> 15), t | 1);
		t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
		return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
	};
}

// Normalize current node positions to the bbox of the supplied nodes. Nodes
// without numeric x/y are skipped. Returns `[0..1]` floats.
export function normalizePositions(
	nodes: PositionedNode[]
): Record<string, [number, number]> {
	const positioned = nodes.filter(
		(n): n is { id: string; x: number; y: number } =>
			Number.isFinite(n.x) && Number.isFinite(n.y)
	);
	if (positioned.length === 0) return {};

	let minX = Infinity;
	let minY = Infinity;
	let maxX = -Infinity;
	let maxY = -Infinity;
	for (const n of positioned) {
		if (n.x < minX) minX = n.x;
		if (n.x > maxX) maxX = n.x;
		if (n.y < minY) minY = n.y;
		if (n.y > maxY) maxY = n.y;
	}
	const dx = maxX - minX || 1;
	const dy = maxY - minY || 1;

	const out: Record<string, [number, number]> = {};
	for (const n of positioned) {
		out[n.id] = [(n.x - minX) / dx, (n.y - minY) / dy];
	}
	return out;
}

// Inverse of `normalizePositions`. Spreads stored `[0..1]` coords across the
// canvas with a margin so node bodies don't clip the edges. Mutates the
// passed-in nodes; nodes whose id isn't in `layout` are left untouched so the
// simulation will place them.
export function applyPositions(
	nodes: PositionedNode[],
	layout: LayoutPayload | null | undefined,
	canvas: Sized,
	opts: { margin?: number } = {}
): number {
	if (!layout || !layout.nodes) return 0;
	const margin = opts.margin ?? 60;
	const innerW = Math.max(1, canvas.width - 2 * margin);
	const innerH = Math.max(1, canvas.height - 2 * margin);
	let applied = 0;
	for (const n of nodes) {
		const p = layout.nodes[n.id];
		if (!p) continue;
		const [nx, ny] = p;
		if (!Number.isFinite(nx) || !Number.isFinite(ny)) continue;
		n.x = margin + nx * innerW;
		n.y = margin + ny * innerH;
		applied++;
	}
	return applied;
}

// Stable scope strings used by both graph views. The exom view uses the
// exom's slash-path (or :: path) verbatim; the global graph uses `_global`.
export const GLOBAL_GRAPH_SCOPE = '_global';
