<script lang="ts">
	import { browser } from '$app/environment';
	import * as d3 from 'd3';
	import { Maximize2, RefreshCw, ZoomIn, ZoomOut } from '@lucide/svelte';
	import { tick, untrack } from 'svelte';

	import {
		collectExomNodes,
		graphViz,
		hideAllGraphExoms,
		showAllGraphExoms
	} from '$lib/graphExomVisibility.svelte';
	import { fetchRelationGraph, type RelationGraphResponse } from '$lib/exomem.svelte';

	let graphCache = $state<Record<string, RelationGraphResponse>>({});

	let svgEl = $state<SVGSVGElement | null>(null);
	let graphError = $state<string | null>(null);
	let graphLoading = $state(false);
	let zoomBehavior = $state<d3.ZoomBehavior<SVGSVGElement, unknown> | null>(null);

	const CHART_PALETTE_FALLBACK = [
		'#2563eb',
		'#059669',
		'#d97706',
		'#dc2626',
		'#7c3aed',
		'#0891b2',
		'#be185d',
		'#4f46e5',
		'#0d9488',
		'#ea580c',
		'#6d28d9',
		'#15803d',
		'#b91c1c',
		'#1d4ed8',
		'#a16207'
	];

	function getChartPalette(): string[] {
		if (typeof document === 'undefined') return Array(10).fill('#888');
		const style = getComputedStyle(document.documentElement);
		return Array.from({ length: 10 }, (_, i) => {
			const raw = style.getPropertyValue(`--chart-${i + 1}`).trim();
			return raw || CHART_PALETTE_FALLBACK[i % CHART_PALETTE_FALLBACK.length];
		});
	}

	const visibleExomPaths = $derived.by(() => {
		const r = graphViz.treeRoot;
		if (!r) return [] as string[];
		return collectExomNodes(r)
			.map((e) => e.path)
			.filter((p) => graphViz.exomVis[p]);
	});

	const visibleExomCount = $derived(visibleExomPaths.length);

	type MergedNode = {
		id: string;
		label: string;
		degree: number;
		exoms: Set<string>;
		primaryExom: string;
	};

	type MergedEdge = {
		source: string;
		target: string;
		predicate: string;
		label: string;
		kind: 'base' | 'derived';
		exom: string;
	};

	function mergeGraphs(graphs: Map<string, RelationGraphResponse>): {
		nodes: MergedNode[];
		edges: MergedEdge[];
	} {
		const nodeMap = new Map<
			string,
			{ id: string; label: string; degree: number; exoms: Set<string> }
		>();
		const edges: MergedEdge[] = [];
		for (const [exom, graph] of graphs) {
			for (const n of graph.nodes) {
				const existing = nodeMap.get(n.id);
				if (existing) {
					existing.degree += n.degree;
					existing.exoms.add(exom);
				} else {
					nodeMap.set(n.id, {
						id: n.id,
						label: n.label,
						degree: n.degree,
						exoms: new Set([exom])
					});
				}
			}
			for (const e of graph.edges) {
				edges.push({
					source: e.source,
					target: e.target,
					predicate: e.predicate,
					label: e.label,
					kind: e.kind,
					exom
				});
			}
		}
		const nodes: MergedNode[] = Array.from(nodeMap.values()).map((n) => {
			const sorted = Array.from(n.exoms).sort();
			return {
				...n,
				primaryExom: sorted[0] ?? ''
			};
		});
		return { nodes, edges };
	}

	const mergedGraph = $derived.by(() => {
		const m = new Map<string, RelationGraphResponse>();
		for (const p of visibleExomPaths) {
			const g = graphCache[p];
			if (g) m.set(p, g);
		}
		return mergeGraphs(m);
	});

	const totalNodes = $derived(mergedGraph.nodes.length);
	const totalEdges = $derived(mergedGraph.edges.length);

	const exomColorMap = $derived.by(() => {
		const palette = getChartPalette();
		const sorted = [...visibleExomPaths].sort();
		const map = new Map<string, string>();
		sorted.forEach((p, i) => map.set(p, palette[i % palette.length]));
		return map;
	});

	const inFlight = new Set<string>();
	$effect(() => {
		if (!browser) return;
		const paths = visibleExomPaths;

		untrack(() => {
			const missing = paths.filter(
				(p) => graphCache[p] === undefined && !inFlight.has(p)
			);
			if (paths.length === 0) {
				graphLoading = false;
				return;
			}
			if (missing.length === 0) {
				if (inFlight.size === 0) graphLoading = false;
				return;
			}
			graphLoading = true;
			graphError = null;
			for (const p of missing) {
				inFlight.add(p);
				void (async () => {
					try {
						const g = await fetchRelationGraph(p);
						graphCache = { ...graphCache, [p]: g };
					} catch (e) {
						graphError = e instanceof Error ? e.message : 'Failed to load graph';
					} finally {
						inFlight.delete(p);
						if (inFlight.size === 0) graphLoading = false;
					}
				})();
			}
		});
	});

	const DEFAULTS = {
		linkDistance: 120,
		chargeStrength: -300,
		collisionRadius: 30,
		nodeScale: 1.0,
		labelSize: 11,
		edgeLabelSize: 9,
		linkWidth: 1.5
	};

	type GNode = d3.SimulationNodeDatum & MergedNode;
	type GEdge = d3.SimulationLinkDatum<GNode> & MergedEdge;

	let simulation: d3.Simulation<GNode, GEdge> | null = null;
	let svgNodeSelection: d3.Selection<SVGCircleElement, GNode, SVGGElement, unknown> | null = null;
	let svgNodeLabelSelection: d3.Selection<SVGTextElement, GNode, SVGGElement, unknown> | null = null;
	let svgEdgeLabelSelection: d3.Selection<SVGTextElement, GEdge, SVGGElement, unknown> | null = null;
	let svgLinkSelection: d3.Selection<SVGLineElement, GEdge, SVGGElement, unknown> | null = null;

	function nodeRadius(d: GNode): number {
		return Math.max(6, Math.min(18, 4 + d.degree * 1.5)) * DEFAULTS.nodeScale;
	}

	function safeMarkerId(part: string): string {
		return part.replace(/[^a-zA-Z0-9_]/g, '_');
	}

	function renderGraph(data: { nodes: MergedNode[]; edges: MergedEdge[] }, colors: Map<string, string>) {
		if (!svgEl) return;
		simulation?.stop();
		const root = svgEl;
		const svg = d3.select(root);
		svg.selectAll('*').remove();

		const width = root.clientWidth;
		const height = root.clientHeight;

		const g = svg.append('g');
		const zoom = d3
			.zoom<SVGSVGElement, unknown>()
			.scaleExtent([0.1, 8])
			.on('zoom', (event) => g.attr('transform', event.transform));
		svg.call(zoom);
		const defs = svg.append('defs');
		const markerKeys = new Set<string>();
		for (const e of data.edges) {
			const mk = `${e.exom}::${e.predicate}`;
			if (markerKeys.has(mk)) continue;
			markerKeys.add(mk);
			const fill = colors.get(e.exom) ?? '#888';
			defs
				.append('marker')
				.attr('id', `arrow-${safeMarkerId(e.exom)}_${safeMarkerId(e.predicate)}`)
				.attr('viewBox', '0 -5 10 10')
				.attr('refX', 22)
				.attr('refY', 0)
				.attr('markerWidth', 6)
				.attr('markerHeight', 6)
				.attr('orient', 'auto')
				.append('path')
				.attr('d', 'M0,-5L10,0L0,5')
				.attr('fill', fill);
		}

		const nodes: GNode[] = data.nodes.map((n) => ({ ...n }));
		const edges: GEdge[] = data.edges.map((e) => ({ ...e }));

		simulation = d3
			.forceSimulation<GNode>(nodes)
			.force(
				'link',
				d3.forceLink<GNode, GEdge>(edges).id((d) => d.id).distance(DEFAULTS.linkDistance)
			)
			.force('charge', d3.forceManyBody().strength(DEFAULTS.chargeStrength))
			.force('center', d3.forceCenter(width / 2, height / 2))
			.force('collision', d3.forceCollide().radius(DEFAULTS.collisionRadius * DEFAULTS.nodeScale));

		svgLinkSelection = g
			.append('g')
			.selectAll<SVGLineElement, GEdge>('line')
			.data(edges)
			.join('line')
			.attr('stroke', (d) => colors.get(d.exom) ?? '#888')
			.attr('stroke-width', DEFAULTS.linkWidth)
			.attr('stroke-opacity', 0.6)
			.attr('stroke-dasharray', (d) => (d.predicate === 'dependency' ? '6 3' : null))
			.attr(
				'marker-end',
				(d) => `url(#arrow-${safeMarkerId(d.exom)}_${safeMarkerId(d.predicate)})`
			);

		svgEdgeLabelSelection = g
			.append('g')
			.selectAll<SVGTextElement, GEdge>('text')
			.data(edges)
			.join('text')
			.attr('font-size', DEFAULTS.edgeLabelSize)
			.attr('fill', (d) => colors.get(d.exom) ?? '#888')
			.attr('text-anchor', 'middle')
			.attr('dy', -4)
			.text((d) => d.predicate);

		svgNodeSelection = g
			.append('g')
			.selectAll<SVGCircleElement, GNode>('circle')
			.data(nodes)
			.join('circle')
			.attr('r', (d) => nodeRadius(d))
			.attr('fill', (d) => colors.get(d.primaryExom) ?? '#1e293b')
			.attr('stroke', '#94a3b8')
			.attr('stroke-width', 1.5)
			.attr('cursor', 'grab')
			.call(
				d3
					.drag<SVGCircleElement, GNode>()
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
					})
			);

		svgNodeLabelSelection = g
			.append('g')
			.selectAll<SVGTextElement, GNode>('text')
			.data(nodes)
			.join('text')
			.attr('font-size', DEFAULTS.labelSize)
			.attr('font-weight', 500)
			.attr('fill', '#e2e8f0')
			.attr('text-anchor', 'middle')
			.attr('dy', (d) => nodeRadius(d) + DEFAULTS.labelSize + 3)
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
	}

	$effect(() => {
		if (!browser || !svgEl) return;
		const data = mergedGraph;
		const colors = exomColorMap;
		void tick().then(() => {
			if (data.nodes.length === 0) {
				simulation?.stop();
				simulation = null;
				d3.select(svgEl!).selectAll('*').remove();
				return;
			}
			renderGraph(data, colors);
		});
	});

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

	function reloadGraphs() {
		const paths = visibleExomPaths;
		const next = { ...graphCache };
		for (const p of paths) delete next[p];
		graphCache = next;
	}
</script>

<div class="flex h-full min-h-0 flex-1 flex-col overflow-hidden bg-background">
	<div class="flex items-center justify-between gap-2 border-b border-border px-3 py-1.5">
		<div class="flex min-w-0 flex-wrap items-center gap-2 text-xs text-muted-foreground">
			<span class="text-[0.65rem] uppercase tracking-wide">Exom visibility</span>
			<div class="flex gap-1">
				<button
					type="button"
					title="Show all"
					onclick={showAllGraphExoms}
					class="rounded px-1.5 py-0.5 text-[0.65rem] text-muted-foreground hover:bg-card hover:text-foreground"
				>
					All
				</button>
				<button
					type="button"
					title="Hide all"
					onclick={hideAllGraphExoms}
					class="rounded px-1.5 py-0.5 text-[0.65rem] text-muted-foreground hover:bg-card hover:text-foreground"
				>
					None
				</button>
			</div>
			<span class="text-border">·</span>
			<span class="tabular-nums">{totalNodes} nodes</span>
			<span class="text-border">·</span>
			<span class="tabular-nums">{totalEdges} edges</span>
			<span class="text-border">·</span>
			<span>{visibleExomCount} exoms</span>
		</div>
		<div class="flex items-center gap-0.5">
			<button
				type="button"
				class="flex size-7 items-center justify-center rounded text-muted-foreground hover:bg-card hover:text-foreground"
				onclick={zoomIn}
				title="Zoom in"
			>
				<ZoomIn class="size-3.5" />
			</button>
			<button
				type="button"
				class="flex size-7 items-center justify-center rounded text-muted-foreground hover:bg-card hover:text-foreground"
				onclick={zoomOut}
				title="Zoom out"
			>
				<ZoomOut class="size-3.5" />
			</button>
			<button
				type="button"
				class="flex size-7 items-center justify-center rounded text-muted-foreground hover:bg-card hover:text-foreground"
				onclick={zoomFit}
				title="Reset view"
			>
				<Maximize2 class="size-3.5" />
			</button>
			<button
				type="button"
				class="flex size-7 items-center justify-center rounded text-muted-foreground hover:bg-card hover:text-foreground"
				onclick={reloadGraphs}
				disabled={graphLoading}
				title="Reload"
			>
				<RefreshCw class="size-3.5 {graphLoading ? 'animate-spin' : ''}" />
			</button>
		</div>
	</div>

	<div class="relative min-h-0 flex-1 overflow-hidden">
		{#if graphError && totalNodes === 0}
			<div
				class="flex h-full min-h-[200px] items-center justify-center px-4 text-center text-sm text-destructive"
			>
				{graphError}
			</div>
		{:else if graphLoading && visibleExomPaths.length > 0 && totalNodes === 0}
			<div
				class="flex h-full min-h-[200px] items-center justify-center text-sm text-muted-foreground"
			>
				<RefreshCw class="mr-2 size-5 animate-spin" />
				Loading graphs…
			</div>
		{:else}
			<svg
				bind:this={svgEl}
				class="block h-full min-h-[320px] w-full bg-card"
			></svg>
		{/if}
	</div>
</div>
