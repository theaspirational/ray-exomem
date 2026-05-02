<script lang="ts">
	import { browser } from '$app/environment';
	import * as d3 from 'd3';
	import {
		RefreshCw, ZoomIn, ZoomOut, Maximize2,
		Settings2, Eye, EyeOff, RotateCcw, X
	} from '@lucide/svelte';

	import { Badge } from '$lib/components/ui/badge';
	import { Button } from '$lib/components/ui/button';
	import { Card } from '$lib/components/ui/card';
	import {
		fetchExomemSchema,
		fetchRelationGraph,
		fetchEntityFacts,
		fetchGraphLayout,
		saveGraphLayout,
		type RelationGraphResponse
	} from '$lib/exomem.svelte';
	import type { ExomemSchemaResponse, FactEntry } from '$lib/types';
	import {
		GRAPH_LAYOUT_VERSION,
		applyPositions,
		normalizePositions,
		seededRng,
		type LayoutPayload
	} from '$lib/graphLayout';

	let { exomPath, branch = 'main' }: { exomPath: string; branch?: string } = $props();

	const CHART_PALETTE_FALLBACK = [
		'#2563eb', '#059669', '#d97706', '#dc2626', '#7c3aed',
		'#0891b2', '#be185d', '#4f46e5', '#0d9488', '#ea580c',
		'#6d28d9', '#15803d', '#b91c1c', '#1d4ed8', '#a16207'
	];

	function getChartPalette(): string[] {
		if (typeof document === 'undefined') return Array(10).fill('#888');
		const style = getComputedStyle(document.documentElement);
		return Array.from({ length: 10 }, (_, i) => {
			const raw = style.getPropertyValue(`--chart-${i + 1}`).trim();
			return raw || CHART_PALETTE_FALLBACK[i % CHART_PALETTE_FALLBACK.length];
		});
	}

	// --- State ---
	let svgEl = $state<SVGSVGElement | null>(null);
	let graph = $state<RelationGraphResponse | null>(null);
	let schema = $state<ExomemSchemaResponse | null>(null);
	let loading = $state(true);
	let error = $state<string | null>(null);
	let hoveredNode = $state<string | null>(null);
	let selectedNode = $state<string | null>(null);
	let selectedNodeFacts = $state<FactEntry[]>([]);
	let selectedNodeLoading = $state(false);
	let factFilterPredicate = $state<string | null>(null);
	let activePredFilters = $state<Set<string>>(new Set());
	let filtersInitialized = $state(false);
	let zoomBehavior: d3.ZoomBehavior<SVGSVGElement, unknown> | null = null;
	let showControls = $state(false);

	const systemPredicateSet = $derived(new Set(schema?.ontology?.system_attributes.map((attr) => attr.name) ?? []));
	const coordinationPredicateSet = $derived(new Set(schema?.ontology?.coordination_attributes.map((attr) => attr.name) ?? []));
	const userPredicateSet = $derived(new Set(schema?.ontology?.user_predicates ?? []));

	// --- Entity panel derived state ---
	const selectedNodeEdges = $derived.by(() => {
		if (!graph || !selectedNode) return [];
		return graph.edges.filter((e) => e.source === selectedNode || e.target === selectedNode
			|| (typeof e.source === 'object' && (e.source as any).id === selectedNode)
			|| (typeof e.target === 'object' && (e.target as any).id === selectedNode));
	});

	const selectedNodePredicates = $derived.by(() => {
		const facts = selectedNodeFacts;
		const preds = new Set(facts.map((f) => f.predicate));
		return Array.from(preds).sort();
	});

	const filteredFacts = $derived(() => {
		if (!factFilterPredicate) return selectedNodeFacts;
		return selectedNodeFacts.filter((f) => f.predicate === factFilterPredicate);
	});

	const selectedFactCounts = $derived.by(() => ({
		user: selectedNodeFacts.filter((f) => userPredicateSet.has(f.predicate)).length,
		system: selectedNodeFacts.filter((f) => systemPredicateSet.has(f.predicate)).length,
		coordination: selectedNodeFacts.filter((f) => coordinationPredicateSet.has(f.predicate)).length
	}));

	function predicateKind(predicate: string): 'user' | 'system' | 'coordination' | 'derived' {
		if (coordinationPredicateSet.has(predicate)) return 'coordination';
		if (systemPredicateSet.has(predicate)) return 'system';
		if (userPredicateSet.has(predicate)) return 'user';
		return 'derived';
	}

	function predicateBadgeClass(predicate: string): string {
		switch (predicateKind(predicate)) {
			case 'system':
				return 'text-rule-accent';
			case 'coordination':
				return 'text-contra';
			case 'user':
				return 'text-fact-base';
			default:
				return 'text-fact-derived';
		}
	}

	async function selectEntity(entityId: string) {
		if (selectedNode === entityId) {
			// Toggle off
			selectedNode = null;
			selectedNodeFacts = [];
			factFilterPredicate = null;
			highlightSelectedNode(null);
			return;
		}
		selectedNode = entityId;
		selectedNodeLoading = true;
		factFilterPredicate = null;
		highlightSelectedNode(entityId);
		try {
			selectedNodeFacts = await fetchEntityFacts(entityId, exomPath, branch);
		} catch {
			selectedNodeFacts = [];
		}
		selectedNodeLoading = false;
	}

	function closeEntityPanel() {
		selectedNode = null;
		selectedNodeFacts = [];
		factFilterPredicate = null;
		highlightSelectedNode(null);
	}

	function highlightSelectedNode(entityId: string | null) {
		if (!svgNodeSelection) return;
		svgNodeSelection
			.attr('stroke', (d) => d.id === entityId ? '#f59e0b' : '#94a3b8')
			.attr('stroke-width', (d) => d.id === entityId ? 3 : 1.5);
	}

	// --- Graph tuning parameters ---
	const DEFAULTS = {
		linkDistance: 120,
		chargeStrength: -300,
		collisionRadius: 30,
		nodeScale: 1.0,
		labelSize: 11,
		edgeLabelSize: 9,
		showNodeLabels: true,
		showEdgeLabels: true,
		linkWidth: 1.5
	};

	let linkDistance = $state(DEFAULTS.linkDistance);
	let chargeStrength = $state(DEFAULTS.chargeStrength);
	let collisionRadius = $state(DEFAULTS.collisionRadius);
	let nodeScale = $state(DEFAULTS.nodeScale);
	let labelSize = $state(DEFAULTS.labelSize);
	let edgeLabelSize = $state(DEFAULTS.edgeLabelSize);
	let showNodeLabels = $state(DEFAULTS.showNodeLabels);
	let showEdgeLabels = $state(DEFAULTS.showEdgeLabels);
	let linkWidth = $state(DEFAULTS.linkWidth);

	// Live refs for simulation updates
	let simulation: d3.Simulation<GNode, GEdge> | null = null;
	let svgNodeSelection: d3.Selection<SVGCircleElement, GNode, SVGGElement, unknown> | null = null;
	let svgNodeLabelSelection: d3.Selection<SVGTextElement, GNode, SVGGElement, unknown> | null = null;
	let svgEdgeLabelSelection: d3.Selection<SVGTextElement, GEdge, SVGGElement, unknown> | null = null;
	let svgLinkSelection: d3.Selection<SVGLineElement, GEdge, SVGGElement, unknown> | null = null;

	// Layout persistence: scope = exom path; positions stored normalized to bbox.
	const layoutScope = $derived(`exom:${exomPath}`);
	let savedLayout: LayoutPayload | null = null;
	let saveTimer: ReturnType<typeof setTimeout> | null = null;
	let activeNodes: GNode[] = [];
	let viewportState: { x: number; y: number; k: number } | null = null;
	// Suppresses persistence while we apply a saved layout to state vars.
	let applyingSaved = false;

	type GNode = d3.SimulationNodeDatum & { id: string; label: string; degree: number };
	type GEdge = d3.SimulationLinkDatum<GNode> & { label: string; predicate: string; kind: string };

	const predicates = $derived(() => {
		if (!graph) return [];
		const set = new Set(graph.edges.map((e) => e.predicate));
		return Array.from(set).sort();
	});

	const predicateColor = $derived(() => {
		const preds = predicates();
		const palette = getChartPalette();
		const map = new Map<string, string>();
		preds.forEach((p, i) => map.set(p, palette[i % palette.length]));
		return map;
	});

	// Whether any filter is actively narrowing the view
	const isFiltering = $derived(() => {
		const preds = predicates();
		if (preds.length === 0) return false;
		return activePredFilters.size < preds.length;
	});

	// Visible edges and nodes based on active predicate filters
	const filteredEdges = $derived(() => {
		if (!graph) return [];
		if (!isFiltering()) return graph.edges;
		return graph.edges.filter((e) => activePredFilters.has(e.predicate));
	});

	const filteredNodeIds = $derived(() => {
		const edges = filteredEdges();
		const ids = new Set<string>();
		for (const e of edges) {
			ids.add(typeof e.source === 'string' ? e.source : (e.source as any).id ?? '');
			ids.add(typeof e.target === 'string' ? e.target : (e.target as any).id ?? '');
		}
		return ids;
	});

	function togglePredFilter(pred: string) {
		const next = new Set(activePredFilters);
		if (next.has(pred)) {
			next.delete(pred);
		} else {
			next.add(pred);
		}
		activePredFilters = next;
		applyFilters();
	}

	function selectAllPredicates() {
		activePredFilters = new Set(predicates());
		applyFilters();
	}

	function clearAllPredicates() {
		activePredFilters = new Set();
		applyFilters();
	}

	function applyFilters() {
		const filtering = isFiltering();
		const visibleNodes = filteredNodeIds();
		const activeSet = activePredFilters;

		if (svgLinkSelection) {
			svgLinkSelection
				.attr('stroke-opacity', (d) =>
					!filtering || activeSet.has(d.predicate) ? 0.6 : 0.06
				)
				.attr('stroke-dasharray', (d) =>
					d.predicate === 'dependency' ? '6 3' : null
				);
		}
		if (svgEdgeLabelSelection) {
			svgEdgeLabelSelection.attr('display', (d) =>
				showEdgeLabels && (!filtering || activeSet.has(d.predicate)) ? null : 'none'
			);
		}
		if (svgNodeSelection) {
			svgNodeSelection
				.attr('opacity', (d) =>
					!filtering || visibleNodes.has(d.id) ? 1 : 0.08
				);
		}
		if (svgNodeLabelSelection) {
			svgNodeLabelSelection.attr('display', (d) =>
				showNodeLabels && (!filtering || visibleNodes.has(d.id)) ? null : 'none'
			);
		}
	}

	$effect(() => {
		if (!browser) return;
		exomPath;
		branch;
		void loadGraph();
	});

	async function loadGraph() {
		loading = true;
		error = null;
		try {
			const [graphRes, schemaRes, layoutRes] = await Promise.all([
				fetchRelationGraph(exomPath, branch),
				fetchExomemSchema(exomPath, undefined, branch),
				fetchGraphLayout(layoutScope).catch(() => null)
			]);
			graph = graphRes;
			schema = schemaRes;
			savedLayout = layoutRes;
			viewportState = layoutRes?.viewport ?? null;
			applySavedControls(layoutRes?.controls);
			selectedNode = null;
			selectedNodeFacts = [];
			factFilterPredicate = null;
			loading = false;
			// Initialize filters to show all predicates
			if (!filtersInitialized && graph) {
				activePredFilters = new Set(graph.edges.map((e) => e.predicate));
				filtersInitialized = true;
			}
			await tick();
			if (graph && svgEl) renderGraph(graph);
		} catch (e) {
			error = e instanceof Error ? e.message : 'Failed to load graph';
			loading = false;
		}
	}

	function applySavedControls(c: LayoutPayload['controls'] | undefined) {
		if (!c) return;
		applyingSaved = true;
		try {
			if (typeof c.linkDistance === 'number') linkDistance = c.linkDistance;
			if (typeof c.chargeStrength === 'number') chargeStrength = c.chargeStrength;
			if (typeof c.collisionRadius === 'number') collisionRadius = c.collisionRadius;
			if (typeof c.nodeScale === 'number') nodeScale = c.nodeScale;
			if (typeof c.labelSize === 'number') labelSize = c.labelSize;
			if (typeof c.edgeLabelSize === 'number') edgeLabelSize = c.edgeLabelSize;
			if (typeof c.linkWidth === 'number') linkWidth = c.linkWidth;
			if (typeof c.showNodeLabels === 'boolean') showNodeLabels = c.showNodeLabels;
			if (typeof c.showEdgeLabels === 'boolean') showEdgeLabels = c.showEdgeLabels;
			if (typeof c.showControls === 'boolean') showControls = c.showControls;
		} finally {
			applyingSaved = false;
		}
	}

	function currentControls(): NonNullable<LayoutPayload['controls']> {
		return {
			linkDistance,
			chargeStrength,
			collisionRadius,
			nodeScale,
			labelSize,
			edgeLabelSize,
			linkWidth,
			showNodeLabels,
			showEdgeLabels,
			showControls
		};
	}

	function schedulePersist() {
		if (applyingSaved) return;
		if (saveTimer) clearTimeout(saveTimer);
		saveTimer = setTimeout(() => {
			saveTimer = null;
			void persistLayout();
		}, 750);
	}

	async function persistLayout() {
		// Build positions only if the simulation has run; otherwise reuse the
		// last known node positions to avoid clobbering them with empties.
		const positions =
			activeNodes.length > 0
				? normalizePositions(activeNodes.map((n) => ({ id: n.id, x: n.x, y: n.y })))
				: savedLayout?.nodes ?? {};
		const payload: LayoutPayload = {
			version: GRAPH_LAYOUT_VERSION,
			nodes: positions,
			controls: currentControls()
		};
		if (viewportState) payload.viewport = viewportState;
		try {
			await saveGraphLayout(layoutScope, payload);
			savedLayout = payload;
		} catch {
			// Persistence failures are non-fatal — the layout still works for this session.
		}
	}

	function tick(): Promise<void> {
		return new Promise((resolvePromise) => setTimeout(resolvePromise, 0));
	}

	function nodeRadius(d: GNode): number {
		return Math.max(6, Math.min(18, 4 + d.degree * 1.5)) * nodeScale;
	}

	function resetDefaults() {
		linkDistance = DEFAULTS.linkDistance;
		chargeStrength = DEFAULTS.chargeStrength;
		collisionRadius = DEFAULTS.collisionRadius;
		nodeScale = DEFAULTS.nodeScale;
		labelSize = DEFAULTS.labelSize;
		edgeLabelSize = DEFAULTS.edgeLabelSize;
		showNodeLabels = DEFAULTS.showNodeLabels;
		showEdgeLabels = DEFAULTS.showEdgeLabels;
		linkWidth = DEFAULTS.linkWidth;
		applyForces();
		applyVisuals();
	}

	function applyForces() {
		if (!simulation) return;
		const linkForce = simulation.force('link') as d3.ForceLink<GNode, GEdge> | undefined;
		if (linkForce) linkForce.distance(linkDistance);
		const chargeForce = simulation.force('charge') as d3.ForceManyBody<GNode> | undefined;
		if (chargeForce) chargeForce.strength(chargeStrength);
		const collisionForce = simulation.force('collision') as d3.ForceCollide<GNode> | undefined;
		if (collisionForce) collisionForce.radius(collisionRadius * nodeScale);
		simulation.alpha(0.5).restart();
		schedulePersist();
	}

	function applyVisuals() {
		if (svgNodeSelection) {
			svgNodeSelection.attr('r', (d) => nodeRadius(d));
		}
		if (svgNodeLabelSelection) {
			svgNodeLabelSelection
				.attr('font-size', labelSize)
				.attr('dy', (d) => nodeRadius(d) + labelSize + 3)
				.attr('display', showNodeLabels ? null : 'none');
		}
		if (svgEdgeLabelSelection) {
			svgEdgeLabelSelection
				.attr('font-size', edgeLabelSize)
				.attr('display', showEdgeLabels ? null : 'none');
		}
		if (svgLinkSelection) {
			svgLinkSelection.attr('stroke-width', linkWidth);
		}
		// Update collision radius with new node scale
		if (simulation) {
			const collisionForce = simulation.force('collision') as d3.ForceCollide<GNode> | undefined;
			if (collisionForce) collisionForce.radius(collisionRadius * nodeScale);
		}
		schedulePersist();
	}

	function renderGraph(data: RelationGraphResponse) {
		if (!svgEl) return;
		const root = svgEl;
		const svg = d3.select(root);
		svg.selectAll('*').remove();

		const width = root.clientWidth;
		const height = root.clientHeight;

		// Zoom
		const g = svg.append('g');
		const zoom = d3.zoom<SVGSVGElement, unknown>()
			.scaleExtent([0.1, 8])
			.on('zoom', (event) => {
				g.attr('transform', event.transform);
				const t = event.transform;
				viewportState = { x: t.x, y: t.y, k: t.k };
				schedulePersist();
			});
		svg.call(zoom);
		if (viewportState) {
			applyingSaved = true;
			try {
				const t = d3.zoomIdentity.translate(viewportState.x, viewportState.y).scale(viewportState.k);
				svg.call(zoom.transform, t);
			} finally {
				applyingSaved = false;
			}
		}

		// Arrow markers per predicate
		const preds = new Set(data.edges.map((e) => e.predicate));
		const colorMap = predicateColor();
		const defs = svg.append('defs');
		for (const pred of preds) {
			const color = colorMap.get(pred) ?? '#888';
			defs.append('marker')
				.attr('id', `arrow-${pred.replace(/[^a-zA-Z0-9_]/g, '_')}`)
				.attr('viewBox', '0 -5 10 10')
				.attr('refX', 22)
				.attr('refY', 0)
				.attr('markerWidth', 6)
				.attr('markerHeight', 6)
				.attr('orient', 'auto')
				.append('path')
				.attr('d', 'M0,-5L10,0L0,5')
				.attr('fill', color);
		}

		// Sort by id so d3-force's index-based phyllotaxis seeding produces a
		// stable initial layout for a given input set.
		const nodes: GNode[] = data.nodes
			.map((n) => ({ ...n }))
			.sort((a, b) => (a.id < b.id ? -1 : a.id > b.id ? 1 : 0));
		const edges: GEdge[] = data.edges.map((e) => ({ ...e }));

		// If we have a saved layout, restore positions before the simulation
		// starts. Nodes missing from the layout are left for the simulation
		// to place via the seeded RNG path below.
		applyPositions(
			nodes as { id: string; x?: number; y?: number }[],
			savedLayout,
			{ width, height }
		);
		activeNodes = nodes;

		simulation = d3
			.forceSimulation<GNode>(nodes)
			.randomSource(seededRng(layoutScope))
			.force(
				'link',
				d3.forceLink<GNode, GEdge>(edges).id((d) => d.id).distance(linkDistance)
			)
			.force('charge', d3.forceManyBody().strength(chargeStrength))
			.force('center', d3.forceCenter(width / 2, height / 2))
			.force('collision', d3.forceCollide().radius(collisionRadius * nodeScale));

		// Edges
		svgLinkSelection = g
			.append('g')
			.selectAll<SVGLineElement, GEdge>('line')
			.data(edges)
			.join('line')
			.attr('stroke', (d) => colorMap.get(d.predicate) ?? '#888')
			.attr('stroke-width', linkWidth)
			.attr('stroke-opacity', 0.6)
			.attr('stroke-dasharray', (d) => d.predicate === 'dependency' ? '6 3' : null)
			.attr('marker-end', (d) => `url(#arrow-${d.predicate.replace(/[^a-zA-Z0-9_]/g, '_')})`);

		// Edge labels
		svgEdgeLabelSelection = g
			.append('g')
			.selectAll<SVGTextElement, GEdge>('text')
			.data(edges)
			.join('text')
			.attr('font-size', edgeLabelSize)
			.attr('fill', (d) => colorMap.get(d.predicate) ?? '#888')
			.attr('text-anchor', 'middle')
			.attr('dy', -4)
			.attr('display', showEdgeLabels ? null : 'none')
			.text((d) => d.predicate);

		// Nodes
		svgNodeSelection = g
			.append('g')
			.selectAll<SVGCircleElement, GNode>('circle')
			.data(nodes)
			.join('circle')
			.attr('r', (d) => nodeRadius(d))
			.attr('fill', '#1e293b')
			.attr('stroke', '#94a3b8')
			.attr('stroke-width', 1.5)
			.attr('cursor', 'grab')
			.on('mouseenter', (_, d) => { hoveredNode = d.id; })
			.on('mouseleave', () => { hoveredNode = null; })
			.on('click', (event, d) => { event.stopPropagation(); selectEntity(d.id); })
			.call(
				d3.drag<SVGCircleElement, GNode>()
					.on('start', (event, d) => {
						if (!event.active) simulation!.alphaTarget(0.3).restart();
						d.fx = d.x;
						d.fy = d.y;
					})
					.on('drag', (event, d) => {
						d.fx = event.x;
						d.fy = event.y;
					})
					.on('end', (event, d) => {
						if (!event.active) simulation!.alphaTarget(0);
						d.fx = null;
						d.fy = null;
						schedulePersist();
					})
			);

		// Node labels
		svgNodeLabelSelection = g
			.append('g')
			.selectAll<SVGTextElement, GNode>('text')
			.data(nodes)
			.join('text')
			.attr('font-size', labelSize)
			.attr('font-weight', 500)
			.attr('fill', '#e2e8f0')
			.attr('text-anchor', 'middle')
			.attr('dy', (d) => nodeRadius(d) + labelSize + 3)
			.attr('display', showNodeLabels ? null : 'none')
			.text((d) => d.label);

		const link = svgLinkSelection;
		const linkLabel = svgEdgeLabelSelection;
		const node = svgNodeSelection;
		const nodeLabel = svgNodeLabelSelection;

		simulation.on('tick', () => {
			link
				.attr('x1', (d) => (d.source as GNode).x!)
				.attr('y1', (d) => (d.source as GNode).y!)
				.attr('x2', (d) => (d.target as GNode).x!)
				.attr('y2', (d) => (d.target as GNode).y!);

			linkLabel
				.attr('x', (d) => ((d.source as GNode).x! + (d.target as GNode).x!) / 2)
				.attr('y', (d) => ((d.source as GNode).y! + (d.target as GNode).y!) / 2);

			node.attr('cx', (d) => d.x!).attr('cy', (d) => d.y!);
			nodeLabel.attr('x', (d) => d.x!).attr('y', (d) => d.y!);
		});

		zoomBehavior = zoom;

		// Apply any active predicate filters to the freshly rendered graph
		applyFilters();
	}

	function zoomIn() {
		if (!zoomBehavior || !svgEl) return;
		d3.select(svgEl).transition().duration(300).call(zoomBehavior.scaleBy, 1.5);
	}
	function zoomOut() {
		if (!zoomBehavior || !svgEl) return;
		d3.select(svgEl).transition().duration(300).call(zoomBehavior.scaleBy, 0.67);
	}
	function zoomFit() {
		if (!zoomBehavior || !svgEl) return;
		d3.select(svgEl).transition().duration(500).call(zoomBehavior.transform, d3.zoomIdentity);
	}
</script>

<div class="flex h-[min(72vh,640px)] min-h-[360px] flex-col">
	<!-- Minimal toolbar -->
	<div class="flex items-center justify-between border-b border-border/60 px-3 py-1.5">
		<div class="flex items-center gap-2 text-xs text-muted-foreground">
			{#if graph}
				<span class="tabular-nums">{graph.summary.node_count} nodes</span>
				<span class="text-muted-foreground/50">·</span>
				<span class="tabular-nums">{graph.summary.edge_count} edges</span>
			{/if}
		</div>
		<div class="flex items-center gap-0.5">
			<button type="button" class="flex size-7 items-center justify-center rounded text-foreground/60 hover:bg-secondary hover:text-foreground" onclick={() => { showControls = !showControls; schedulePersist(); }} title="Toggle controls">
				<Settings2 class="size-3.5" />
			</button>
			<button type="button" class="flex size-7 items-center justify-center rounded text-foreground/60 hover:bg-secondary hover:text-foreground" onclick={zoomIn} title="Zoom in">
				<ZoomIn class="size-3.5" />
			</button>
			<button type="button" class="flex size-7 items-center justify-center rounded text-foreground/60 hover:bg-secondary hover:text-foreground" onclick={zoomOut} title="Zoom out">
				<ZoomOut class="size-3.5" />
			</button>
			<button type="button" class="flex size-7 items-center justify-center rounded text-foreground/60 hover:bg-secondary hover:text-foreground" onclick={zoomFit} title="Reset view">
				<Maximize2 class="size-3.5" />
			</button>
			<button type="button" class="flex size-7 items-center justify-center rounded text-foreground/60 hover:bg-secondary hover:text-foreground" onclick={loadGraph} disabled={loading} title="Reload">
				<RefreshCw class="size-3.5 {loading ? 'animate-spin' : ''}" />
			</button>
		</div>
	</div>

	<div class="relative flex min-h-0 min-w-0 flex-1 overflow-hidden">
		<!-- Graph canvas -->
		<div class="flex min-h-0 min-w-0 flex-1 flex-col">
			{#if error}
				<div class="flex h-full min-h-[200px] items-center justify-center">
					<Card class="max-w-md border-destructive/30 bg-destructive/10">
						<p class="text-sm text-destructive">{error}</p>
						<Button variant="outline" size="sm" onclick={loadGraph} class="mt-3">Retry</Button>
					</Card>
				</div>
			{:else if loading}
				<div class="flex h-full min-h-[200px] items-center justify-center text-muted-foreground">
					<RefreshCw class="mr-2 size-5 animate-spin" />
					Loading graph...
				</div>
			{:else}
				<div class="min-h-0 flex-1 overflow-x-auto">
					<svg
						bind:this={svgEl}
						class="block h-full min-h-[400px] w-full"
						style="min-width: 600px; background: oklch(0.15 0.01 260);"
					></svg>
				</div>
			{/if}
		</div>

		<!-- Controls panel -->
		{#if showControls && graph}
			<div class="w-56 shrink-0 overflow-y-auto border-l border-border/60 bg-card/95 p-3 text-xs">
				<!-- Forces section -->
				<div class="flex items-center justify-between mb-3">
					<p class="text-xs font-semibold uppercase tracking-wide text-muted-foreground">Forces</p>
					<button onclick={resetDefaults} class="text-xs text-muted-foreground hover:text-foreground transition-colors flex items-center gap-1" title="Reset all to defaults">
						<RotateCcw class="size-3" />
						Reset
					</button>
				</div>

				<label class="graph-control">
					<span class="graph-control-header">
						<span>Link Distance</span>
						<span class="tabular-nums text-muted-foreground">{linkDistance}</span>
					</span>
					<input type="range" min="30" max="400" step="10" bind:value={linkDistance}
						oninput={() => applyForces()} />
				</label>

				<label class="graph-control">
					<span class="graph-control-header">
						<span>Repulsion</span>
						<span class="tabular-nums text-muted-foreground">{chargeStrength}</span>
					</span>
					<input type="range" min="-1000" max="-30" step="10" bind:value={chargeStrength}
						oninput={() => applyForces()} />
				</label>

				<label class="graph-control">
					<span class="graph-control-header">
						<span>Collision Radius</span>
						<span class="tabular-nums text-muted-foreground">{collisionRadius}</span>
					</span>
					<input type="range" min="5" max="80" step="5" bind:value={collisionRadius}
						oninput={() => applyForces()} />
				</label>

				<!-- Appearance section -->
				<p class="text-xs font-semibold uppercase tracking-wide text-muted-foreground mt-5 mb-3">Appearance</p>

				<label class="graph-control">
					<span class="graph-control-header">
						<span>Node Size</span>
						<span class="tabular-nums text-muted-foreground">{nodeScale.toFixed(1)}x</span>
					</span>
					<input type="range" min="0.3" max="3.0" step="0.1" bind:value={nodeScale}
						oninput={() => { applyVisuals(); applyForces(); }} />
				</label>

				<label class="graph-control">
					<span class="graph-control-header">
						<span>Node Label Size</span>
						<span class="tabular-nums text-muted-foreground">{labelSize}px</span>
					</span>
					<input type="range" min="6" max="24" step="1" bind:value={labelSize}
						oninput={() => applyVisuals()} />
				</label>

				<label class="graph-control">
					<span class="graph-control-header">
						<span>Edge Label Size</span>
						<span class="tabular-nums text-muted-foreground">{edgeLabelSize}px</span>
					</span>
					<input type="range" min="5" max="18" step="1" bind:value={edgeLabelSize}
						oninput={() => applyVisuals()} />
				</label>

				<label class="graph-control">
					<span class="graph-control-header">
						<span>Edge Width</span>
						<span class="tabular-nums text-muted-foreground">{linkWidth.toFixed(1)}</span>
					</span>
					<input type="range" min="0.5" max="5" step="0.5" bind:value={linkWidth}
						oninput={() => applyVisuals()} />
				</label>

				<!-- Visibility toggles -->
				<p class="text-xs font-semibold uppercase tracking-wide text-muted-foreground mt-5 mb-3">Visibility</p>

				<button
					class="graph-toggle"
					onclick={() => { showNodeLabels = !showNodeLabels; applyVisuals(); }}
				>
					{#if showNodeLabels}
						<Eye class="size-3.5 text-foreground" />
					{:else}
						<EyeOff class="size-3.5 text-muted-foreground" />
					{/if}
					<span class:text-muted-foreground={!showNodeLabels}>Node Labels</span>
				</button>

				<button
					class="graph-toggle"
					onclick={() => { showEdgeLabels = !showEdgeLabels; applyVisuals(); }}
				>
					{#if showEdgeLabels}
						<Eye class="size-3.5 text-foreground" />
					{:else}
						<EyeOff class="size-3.5 text-muted-foreground" />
					{/if}
					<span class:text-muted-foreground={!showEdgeLabels}>Edge Labels</span>
				</button>

				<!-- Predicates section -->
				<div class="flex items-center justify-between mt-5 mb-3">
					<p class="text-xs font-semibold uppercase tracking-wide text-muted-foreground">Predicates</p>
					<div class="flex items-center gap-1.5">
						<button
							onclick={selectAllPredicates}
							class="text-[0.65rem] text-muted-foreground hover:text-foreground transition-colors"
							title="Show all predicates"
						>All</button>
						<span class="text-muted-foreground/40 text-[0.65rem]">/</span>
						<button
							onclick={clearAllPredicates}
							class="text-[0.65rem] text-muted-foreground hover:text-foreground transition-colors"
							title="Hide all predicates"
						>None</button>
					</div>
				</div>
				<div class="flex flex-col gap-1.5">
					{#each predicates() as pred (pred)}
						{@const color = predicateColor().get(pred) ?? '#888'}
						{@const count = graph.edges.filter((e) => e.predicate === pred).length}
						{@const active = activePredFilters.has(pred)}
						<button
							class="flex items-center gap-2 rounded-md px-2 py-1.5 text-left text-xs transition-colors hover:bg-muted/40 {active ? 'bg-muted/30' : 'opacity-40'}"
							onclick={() => togglePredFilter(pred)}
						>
							<span
								class="inline-block size-2.5 shrink-0 rounded-full transition-opacity"
								style="background: {color}; opacity: {active ? 1 : 0.3}"
							></span>
							<span class="flex-1 truncate font-medium">{pred}</span>
							<span class="text-[0.6rem] font-mono {predicateBadgeClass(pred)}">{predicateKind(pred)}</span>
							<span class="tabular-nums text-muted-foreground">{count}</span>
						</button>
					{/each}
				</div>
				{#if isFiltering()}
					<p class="mt-2 text-[0.65rem] text-muted-foreground/60">
						{activePredFilters.size} of {predicates().length} shown
					</p>
				{/if}

				{#if hoveredNode && graph}
					{@const inEdges = graph.edges.filter((e) => e.target === hoveredNode || e.source === hoveredNode)}
					<div class="mt-5 border-t border-border/80 pt-4">
						<p class="mb-1 text-xs font-semibold uppercase tracking-wide text-muted-foreground">Hovered</p>
						<p class="text-sm font-medium">{hoveredNode}</p>
						<p class="mt-1 text-xs text-muted-foreground">{inEdges.length} connections</p>
						<div class="mt-2 flex flex-col gap-1">
							{#each inEdges.slice(0, 8) as edge (edge.label + edge.source + edge.target)}
								<p class="truncate text-xs text-muted-foreground">
									<span style="color: {predicateColor().get(edge.predicate) ?? '#888'}">{edge.predicate}</span>
									{' '}
									{edge.source === hoveredNode ? `-> ${edge.target}` : `<- ${edge.source}`}
								</p>
							{/each}
							{#if inEdges.length > 8}
								<p class="text-xs text-muted-foreground/60">+{inEdges.length - 8} more</p>
							{/if}
						</div>
					</div>
				{/if}

				{#if selectedNode}
					<div class="mt-5 border-t border-border/80 pt-4">
						<div class="mb-2 flex items-center justify-between gap-2">
							<p class="text-xs font-semibold uppercase tracking-wide text-muted-foreground">Selected Entity</p>
							<button class="rounded p-1 text-muted-foreground hover:bg-muted/60" onclick={closeEntityPanel}>
								<X class="size-3.5" />
							</button>
						</div>
						<p class="truncate font-mono text-sm text-foreground">{selectedNode}</p>
						<div class="mt-2 flex flex-wrap gap-1.5">
							<Badge variant="outline" class="h-4 px-1.5 text-[10px]">{selectedNodeEdges.length} graph edges</Badge>
							<Badge variant="outline" class="h-4 px-1.5 text-[10px]">{selectedNodeFacts.length} facts</Badge>
							{#if selectedFactCounts.user > 0}
								<Badge variant="outline" class="h-4 px-1.5 text-[10px] text-fact-base">{selectedFactCounts.user} user</Badge>
							{/if}
							{#if selectedFactCounts.system > 0}
								<Badge variant="outline" class="h-4 px-1.5 text-[10px] text-rule-accent">{selectedFactCounts.system} system</Badge>
							{/if}
							{#if selectedFactCounts.coordination > 0}
								<Badge variant="outline" class="h-4 px-1.5 text-[10px] text-contra">{selectedFactCounts.coordination} coordination</Badge>
							{/if}
						</div>

						{#if selectedNodeLoading}
							<div class="mt-3 flex items-center gap-2 text-xs text-muted-foreground">
								<RefreshCw class="size-3 animate-spin" />
								Loading entity facts...
							</div>
						{:else if selectedNodeFacts.length === 0}
							<p class="mt-3 text-xs text-muted-foreground">No sampled facts for this entity in the current graph scope.</p>
						{:else}
							{#if selectedNodePredicates.length > 1}
								<div class="mt-3 flex flex-wrap gap-1">
									<button
										class="rounded-md border border-border/60 px-2 py-1 text-[10px] {factFilterPredicate === null ? 'bg-muted/40 text-foreground' : 'text-muted-foreground'}"
										onclick={() => (factFilterPredicate = null)}
									>
										all
									</button>
									{#each selectedNodePredicates as pred (pred)}
										<button
											class="rounded-md border border-border/60 px-2 py-1 text-[10px] {factFilterPredicate === pred ? 'bg-muted/40 text-foreground' : 'text-muted-foreground'}"
											onclick={() => (factFilterPredicate = pred)}
										>
											{pred}
										</button>
									{/each}
								</div>
							{/if}

							<div class="mt-3 flex flex-col gap-2">
								{#each filteredFacts() as fact, idx (`${fact.predicate}-${fact.terms.join(',')}-${idx}`)}
									<div class="rounded-md border border-border/60 bg-card/50 px-2.5 py-2">
										<div class="flex flex-wrap items-center gap-1.5">
											<span class="font-mono text-[11px] text-foreground">{fact.predicate}</span>
											<Badge variant="outline" class="h-4 px-1.5 text-[10px] {predicateBadgeClass(fact.predicate)}">{predicateKind(fact.predicate)}</Badge>
											{#if fact.branchRole}
												<Badge variant="outline" class="h-4 px-1.5 text-[10px]">{fact.branchRole}</Badge>
											{/if}
										</div>
										<p class="mt-1 font-mono text-[11px] text-muted-foreground">({fact.terms.join(', ')})</p>
										{#if fact.validFrom || fact.validTo}
											<p class="mt-1 text-[10px] text-muted-foreground">
												{fact.validFrom ?? 'unknown'} → {fact.validTo ?? 'open'}
											</p>
										{/if}
										{#if fact.branchOrigin}
											<p class="mt-1 text-[10px] text-muted-foreground">origin branch: <span class="font-mono">{fact.branchOrigin}</span></p>
										{/if}
									</div>
								{/each}
							</div>
						{/if}
					</div>
				{/if}
			</div>
		{/if}
	</div>
</div>

<style>
	.graph-control {
		display: flex;
		flex-direction: column;
		gap: 0.25rem;
		margin-bottom: 0.75rem;
		font-size: 0.75rem;
		cursor: default;
	}
	.graph-control-header {
		display: flex;
		justify-content: space-between;
		align-items: baseline;
	}
	.graph-control input[type='range'] {
		-webkit-appearance: none;
		appearance: none;
		width: 100%;
		height: 4px;
		border-radius: 2px;
		background: oklch(0.3 0.01 260);
		outline: none;
		cursor: pointer;
	}
	.graph-control input[type='range']::-webkit-slider-thumb {
		-webkit-appearance: none;
		appearance: none;
		width: 14px;
		height: 14px;
		border-radius: 50%;
		background: oklch(0.7 0.02 260);
		border: 2px solid oklch(0.5 0.02 260);
		cursor: pointer;
		transition: background 0.15s;
	}
	.graph-control input[type='range']::-webkit-slider-thumb:hover {
		background: oklch(0.85 0.02 260);
	}
	.graph-control input[type='range']::-moz-range-thumb {
		width: 14px;
		height: 14px;
		border-radius: 50%;
		background: oklch(0.7 0.02 260);
		border: 2px solid oklch(0.5 0.02 260);
		cursor: pointer;
	}
	.graph-toggle {
		display: flex;
		align-items: center;
		gap: 0.5rem;
		padding: 0.375rem 0.5rem;
		border-radius: 0.375rem;
		font-size: 0.75rem;
		width: 100%;
		text-align: left;
		transition: background 0.15s;
	}
	.graph-toggle:hover {
		background: oklch(0.25 0.01 260);
	}
</style>
