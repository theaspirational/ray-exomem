import { untrack } from 'svelte';
import type { TreeExom, TreeNode } from '$lib/exomem.svelte';

function collectExomNodes(node: TreeNode): TreeExom[] {
	if (node.kind === 'exom') return [node];
	return node.children.flatMap((c) => collectExomNodes(c));
}

function collectExomPaths(node: TreeNode): string[] {
	if (node.kind === 'exom') return [node.path];
	return node.children.flatMap((c) => collectExomPaths(c));
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

type GraphViz = {
	treeRoot: TreeNode | null;
	exomVis: Record<string, boolean>;
};

export const graphViz = $state<GraphViz>({ treeRoot: null, exomVis: {} });

export function initGraphExomVisFromTree(root: TreeNode) {
	untrack(() => {
		graphViz.treeRoot = root;
		const patch: Record<string, boolean> = { ...graphViz.exomVis };
		for (const e of collectExomNodes(root)) {
			if (!(e.path in patch)) {
				patch[e.path] = e.exom_kind !== 'session';
			}
		}
		graphViz.exomVis = patch;
	});
}

export function toggleGraphExom(path: string) {
	graphViz.exomVis = { ...graphViz.exomVis, [path]: !graphViz.exomVis[path] };
}

export function graphFolderVisState(
	folderPath: string,
	visibility: Record<string, boolean>
): 'all' | 'none' | 'mixed' {
	const r = graphViz.treeRoot;
	if (!r) return 'all';
	const n = findNode(r, folderPath);
	if (!n || n.kind !== 'folder') return 'all';
	const exoms = collectExomPaths(n);
	if (exoms.length === 0) return 'all';
	let vis = 0;
	for (const p of exoms) {
		if (visibility[p]) vis++;
	}
	if (vis === 0) return 'none';
	if (vis === exoms.length) return 'all';
	return 'mixed';
}

export function toggleGraphFolderVisibility(folderPath: string) {
	if (!graphViz.treeRoot) return;
	const n = findNode(graphViz.treeRoot, folderPath);
	if (!n || n.kind !== 'folder') return;
	const exoms = collectExomPaths(n);
	if (exoms.length === 0) return;
	const visCount = exoms.filter((p) => graphViz.exomVis[p]).length;
	const setTo = visCount < exoms.length;
	const patch: Record<string, boolean> = {};
	for (const p of exoms) patch[p] = setTo;
	graphViz.exomVis = { ...graphViz.exomVis, ...patch };
}

export function showAllGraphExoms() {
	if (!graphViz.treeRoot) return;
	const exoms = collectExomNodes(graphViz.treeRoot);
	const patch: Record<string, boolean> = {};
	for (const e of exoms) patch[e.path] = true;
	graphViz.exomVis = { ...graphViz.exomVis, ...patch };
}

export function hideAllGraphExoms() {
	if (!graphViz.treeRoot) return;
	const exoms = collectExomNodes(graphViz.treeRoot);
	const patch: Record<string, boolean> = {};
	for (const e of exoms) patch[e.path] = false;
	graphViz.exomVis = { ...graphViz.exomVis, ...patch };
}

export { collectExomNodes };
