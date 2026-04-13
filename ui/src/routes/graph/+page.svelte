<script lang="ts">
	import { browser } from '$app/environment';
	import * as d3 from 'd3';
	import {
		ChevronRight,
		Circle,
		Eye,
		EyeOff,
		FolderOpen,
		Maximize2,
		RefreshCw,
		ZoomIn,
		ZoomOut
	} from '@lucide/svelte';
	import { tick } from 'svelte';

	import {
		fetchRelationGraph,
		fetchTree,
		type RelationGraphResponse,
		type TreeExom,
		type TreeNode
	} from '$lib/exomem.svelte';

	let treeRoot = $state<TreeNode | null>(null);
	let treeError = $state<string | null>(null);
	let treeLoading = $state(true);

	let visibility = $state<Record<string, boolean>>({});
	let folderOpen = $state<Record<string, boolean>>({});
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

	function exomDotClass(kind: string): string {
		if (kind === 'project_main' || kind === 'project-main') return 'fill-emerald-500 text-emerald-500';
		if (kind === 'session') return 'fill-sky-500 text-sky-500';
		return 'fill-zinc-500 text-zinc-500';
	}

	function findNode(root: TreeNode, path: string): TreeNode | null {
		if (root.path === path) return root;
		if (root.kind !== 'folder') return null;
		for (const c of root.children) {
			const r = findNode(c, path);
			if (r) return r;
		}
		return null;
	}

	function collectExomPaths(node: TreeNode): string[] {
		if (node.kind === 'exom') return [node.path];
		return node.children.flatMap((c) => collectExomPaths(c));
	}

	function collectExomNodes(node: TreeNode): TreeExom[] {
		if (node.kind === 'exom') return [node];
		return node.children.flatMap((c) => collectExomNodes(c));
	}

	function folderIsOpen(path: string): boolean {
		return folderOpen[path] !== false;
	}

	function toggleFolder(path: string) {
		folderOpen = { ...folderOpen, [path]: !folderIsOpen(path) };
	}

	function exomsUnderFolder(folderPath: string): string[] {
		if (!treeRoot) return [];
		const n = findNode(treeRoot, folderPath);
		if (!n || n.kind !== 'folder') return [];
		return collectExomPaths(n);
	}

	function folderVisibility(folderPath: string): 'all' | 'none' | 'mixed' {
		const exoms = exomsUnderFolder(folderPath);
		if (exoms.length === 0) return 'all';
		let vis = 0;
		for (const p of exoms) {
			if (visibility[p]) vis++;
		}
		if (vis === 0) return 'none';
		if (vis === exoms.length) return 'all';
		return 'mixed';
	}

	function toggleFolderVisibility(folderPath: string) {
		const exoms = exomsUnderFolder(folderPath);
		if (exoms.length === 0) return;
		const visCount = exoms.filter((p) => visibility[p]).length;
		const setTo = visCount < exoms.length;
		const patch: Record<string, boolean> = {};
		for (const p of exoms) patch[p] = setTo;
		visibility = { ...visibility, ...patch };
	}

	function toggleExomVisibility(path: string) {
		visibility = { ...visibility, [path]: !visibility[path] };
	}

	function showAllExoms() {
		if (!treeRoot) return;
		const exoms = collectExomNodes(treeRoot);
		const patch: Record<string, boolean> = {};
		for (const e of exoms) patch[e.path] = true;
		visibility = { ...visibility, ...patch };
	}

	function hideAllExoms() {
		if (!treeRoot) return;
		const exoms = collectExomNodes(treeRoot);
		const patch: Record<string, boolean> = {};
		for (const e of exoms) patch[e.path] = false;
		visibility = { ...visibility, ...patch };
	}

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

	const visibleExomPaths = $derived.by(() => {
		if (!treeRoot) return [] as string[];
		return collectExomNodes(treeRoot)
			.map((e) => e.path)
			.filter((p) => visibility[p]);
	});

	const visibleExomCount = $derived(visibleExomPaths.length);

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

	async function loadTree() {
		treeLoading = true;
		treeError = null;
		try {
			const root = await fetchTree(undefined, { depth: 10, archived: true });
			treeRoot = root;
			const patch: Record<string, boolean> = { ...visibility };
			for (const e of collectExomNodes(root)) {
				if (!(e.path in patch)) {
					patch[e.path] = e.exom_kind !== 'session';
				}
			}
			visibility = patch;
		} catch (e) {
			treeRoot = null;
			treeError = e instanceof Error ? e.message : 'Failed to load tree';
		} finally {
			treeLoading = false;
		}
	}

	$effect(() => {
		if (!browser) return;
		void loadTree();
	});

	let fetchGeneration = 0;
	$effect(() => {
		if (!browser) return;
		const paths = visibleExomPaths;
		const gen = ++fetchGeneration;
		const missing = paths.filter((p) => graphCache[p] === undefined);
		if (paths.length === 0) {
			graphLoading = false;
			return;
		}
		if (missing.length === 0) {
			graphLoading = false;
			return;
		}
		graphLoading = true;
		graphError = null;
		void (async () => {
			for (const p of missing) {
				if (gen !== fetchGeneration) return;
				try {
					const g = await fetchRelationGraph(p);
					if (gen !== fetchGeneration) return;
					graphCache = { ...graphCache, [p]: g };
				} catch (e) {
					if (gen !== fetchGeneration) return;
					graphError = e instanceof Error ? e.message : 'Failed to load graph';
				}
			}
			if (gen === fetchGeneration) graphLoading = false;
		})();
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

{#snippet treeRows(node: TreeNode)}
	{#if node.kind === 'folder'}
		{@const open = folderIsOpen(node.path)}
		{@const fv = folderVisibility(node.path)}
		<div class="flex flex-col">
			<div class="flex items-center gap-1 rounded px-1 py-0.5 hover:bg-zinc-800/50">
				<button
					type="button"
					onclick={() => toggleFolder(node.path)}
					class="flex size-5 shrink-0 items-center justify-center rounded text-zinc-500 hover:bg-zinc-800 hover:text-zinc-300"
					aria-expanded={open}
				>
					<ChevronRight class="size-3 transition-transform {open ? 'rotate-90' : ''}" />
				</button>
				<FolderOpen class="size-3 shrink-0 text-amber-500/80" />
				<span class="min-w-0 flex-1 truncate text-zinc-300">{node.name}</span>
				<button
					type="button"
					onclick={() => toggleFolderVisibility(node.path)}
					class="flex size-6 shrink-0 items-center justify-center rounded text-zinc-500 hover:bg-zinc-800 hover:text-zinc-300"
					aria-label="Toggle visibility for folder {node.name}"
				>
					{#if fv === 'all'}
						<Eye class="size-3 text-zinc-400" />
					{:else if fv === 'none'}
						<EyeOff class="size-3 text-zinc-600" />
					{:else}
						<Eye class="size-3 text-zinc-600 opacity-50" />
					{/if}
				</button>
			</div>
			{#if open}
				<div class="ml-2 border-l border-zinc-800 pl-1">
					{#each node.children as child (child.path)}
						{@render treeRows(child)}
					{/each}
				</div>
			{/if}
		</div>
	{:else}
		<div class="flex items-center gap-1 rounded px-1 py-0.5 hover:bg-zinc-800/50">
			<span class="size-3 shrink-0"></span>
			<Circle class="size-2 shrink-0 {exomDotClass(node.exom_kind)}" />
			<span class="min-w-0 flex-1 truncate text-zinc-200">{node.name}</span>
			<button
				type="button"
				onclick={() => toggleExomVisibility(node.path)}
				class="flex size-6 shrink-0 items-center justify-center rounded text-zinc-500 hover:bg-zinc-800 hover:text-zinc-300"
				aria-label="Toggle visibility for {node.name}"
			>
				{#if visibility[node.path]}
					<Eye class="size-3 text-zinc-400" />
				{:else}
					<EyeOff class="size-3 text-zinc-600" />
				{/if}
			</button>
		</div>
	{/if}
{/snippet}

<div class="flex h-full min-h-0 flex-1 overflow-hidden bg-zinc-950">
	<!-- Sidebar -->
	<div
		class="w-56 shrink-0 overflow-y-auto border-r border-zinc-800 bg-zinc-900 p-2 text-xs"
	>
		<div class="mb-2 flex items-center justify-between px-1">
			<span class="text-[0.65rem] uppercase tracking-wide text-zinc-500">Exom Visibility</span>
			<div class="flex gap-1">
				<button
					type="button"
					title="Show all"
					onclick={showAllExoms}
					class="rounded px-1.5 py-0.5 text-[0.65rem] text-zinc-500 hover:bg-zinc-800 hover:text-zinc-300"
				>
					All
				</button>
				<button
					type="button"
					title="Hide all"
					onclick={hideAllExoms}
					class="rounded px-1.5 py-0.5 text-[0.65rem] text-zinc-500 hover:bg-zinc-800 hover:text-zinc-300"
				>
					None
				</button>
			</div>
		</div>
		{#if treeLoading}
			<p class="px-1 text-zinc-500">Loading tree…</p>
		{:else if treeError}
			<p class="px-1 text-red-400">{treeError}</p>
		{:else if treeRoot}
			{@render treeRows(treeRoot)}
		{/if}
	</div>

	<!-- Graph -->
	<div class="flex min-w-0 flex-1 flex-col">
		<div class="flex items-center justify-between border-b border-zinc-800 px-3 py-1.5">
			<div class="flex items-center gap-2 text-xs text-zinc-500">
				<span class="tabular-nums">{totalNodes} nodes</span>
				<span class="text-zinc-700">·</span>
				<span class="tabular-nums">{totalEdges} edges</span>
				<span class="text-zinc-700">·</span>
				<span>{visibleExomCount} exoms</span>
			</div>
			<div class="flex items-center gap-0.5">
				<button
					type="button"
					class="flex size-7 items-center justify-center rounded text-zinc-400 hover:bg-zinc-800 hover:text-zinc-200"
					onclick={zoomIn}
					title="Zoom in"
				>
					<ZoomIn class="size-3.5" />
				</button>
				<button
					type="button"
					class="flex size-7 items-center justify-center rounded text-zinc-400 hover:bg-zinc-800 hover:text-zinc-200"
					onclick={zoomOut}
					title="Zoom out"
				>
					<ZoomOut class="size-3.5" />
				</button>
				<button
					type="button"
					class="flex size-7 items-center justify-center rounded text-zinc-400 hover:bg-zinc-800 hover:text-zinc-200"
					onclick={zoomFit}
					title="Reset view"
				>
					<Maximize2 class="size-3.5" />
				</button>
				<button
					type="button"
					class="flex size-7 items-center justify-center rounded text-zinc-400 hover:bg-zinc-800 hover:text-zinc-200"
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
				<div class="flex h-full min-h-[200px] items-center justify-center px-4 text-center text-sm text-red-400">
					{graphError}
				</div>
			{:else if graphLoading && visibleExomPaths.length > 0 && totalNodes === 0}
				<div
					class="flex h-full min-h-[200px] items-center justify-center text-sm text-zinc-500"
				>
					<RefreshCw class="mr-2 size-5 animate-spin" />
					Loading graphs…
				</div>
			{:else}
				<svg
					bind:this={svgEl}
					class="block h-full min-h-[320px] w-full"
					style="background: oklch(0.15 0.01 260);"
				></svg>
			{/if}
		</div>
	</div>
</div>
