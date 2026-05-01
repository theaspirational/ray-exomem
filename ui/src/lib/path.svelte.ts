/** UI slash path (`work/ath/main`). CLI uses `::` separators. */

export function toSlash(p: string): string {
	return p.replace(/::/g, '/');
}

export function toCli(p: string): string {
	return p.replace(/\//g, '::');
}

export function segments(p: string): string[] {
	return toSlash(p)
		.split('/')
		.map((s) => s.trim())
		.filter((s) => s.length > 0);
}

export function parent(p: string): string | null {
	const segs = segments(p);
	if (segs.length <= 1) return null;
	return segs.slice(0, -1).join('/');
}

export function isProjectMainExomKind(kind: string): boolean {
	return kind === 'project_main' || kind === 'project-main';
}

export function projectFolderName(path: string): string | null {
	const segs = segments(path);
	if (segs.length < 2) return null;
	return segs[segs.length - 2] ?? null;
}

export function treeExomDisplayName(node: {
	name: string;
	path: string;
	exom_kind: string;
}): string {
	if (isProjectMainExomKind(node.exom_kind)) {
		return projectFolderName(node.path) ?? node.name;
	}
	return node.name;
}
