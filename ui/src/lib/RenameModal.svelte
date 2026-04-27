<script lang="ts">
	import { Loader2, RefreshCw } from '@lucide/svelte';
	import { toast } from 'svelte-sonner';
	import { Badge } from '$lib/components/ui/badge/index.js';
	import { Button } from '$lib/components/ui/button/index.js';
	import * as Dialog from '$lib/components/ui/dialog/index.js';
	import { Input } from '$lib/components/ui/input/index.js';
	import { actorPrompt } from '$lib/actorPrompt.svelte';
	import { apiRename, fetchTree, type TreeNode } from '$lib/exomem.svelte';
	import { parent, segments } from '$lib/path.svelte';

	let {
		open = $bindable(false),
		path = '',
		onClose,
		onConfirm
	}: {
		open?: boolean;
		path: string;
		onClose: () => void;
		onConfirm: (newSegment: string) => void;
	} = $props();

	let newSegment = $state('');
	let loading = $state(false);
	let subtree = $state<TreeNode | null>(null);
	let subtreeErr = $state<string | null>(null);
	let subtreeRetry = $state(0);
	let busy = $state(false);

	const FIFTEEN_MS = 15 * 60 * 1000;

	function collectPaths(n: TreeNode): string[] {
		if (n.kind === 'folder') {
			const self = n.path ? [n.path] : [];
			const rest = n.children.flatMap((c) => collectPaths(c));
			return [...self, ...rest];
		}
		return [n.path];
	}

	function newPathAfterRename(oldRoot: string, seg: string, d: string): string {
		const segs = segments(oldRoot);
		const par = parent(oldRoot);
		const newBase = par ? `${par}/${seg}` : seg;
		if (d === oldRoot) return newBase;
		if (d.startsWith(`${oldRoot}/`)) {
			return newBase + d.slice(oldRoot.length);
		}
		return d;
	}

	const previewRowsAll = $derived.by(() => {
		const seg = newSegment.trim();
		if (!path || !seg || !subtree) return [];
		const all = collectPaths(subtree);
		return all
			.filter((d) => d === path || d.startsWith(`${path}/`))
			.map((d) => ({ from: d, to: newPathAfterRename(path, seg, d) }))
			.filter((r) => r.from !== r.to);
	});

	const previewRows = $derived(previewRowsAll.slice(0, 40));

	const previewExtra = $derived(Math.max(0, previewRowsAll.length - previewRows.length));

	type ActiveSession = { rel: string; branches: string[] };

	const activeSessions = $derived.by((): ActiveSession[] => {
		const out: ActiveSession[] = [];
		function isRecent(iso: string): boolean {
			const t = Date.parse(iso);
			if (Number.isNaN(t)) return false;
			return Date.now() - t < FIFTEEN_MS;
		}
		function walk(n: TreeNode) {
			if (n.kind === 'exom') {
				if (n.exom_kind === 'session' && n.last_tx && isRecent(n.last_tx)) {
					const rel =
						path && n.path.startsWith(`${path}/`)
							? n.path.slice(path.length + 1)
							: n.path === path
								? '(this session)'
								: n.path;
					const br = n.branches?.length ? n.branches : ['main'];
					out.push({ rel, branches: br });
				}
				return;
			}
			for (const c of n.children) walk(c);
		}
		if (subtree) walk(subtree);
		return out;
	});

	$effect(() => {
		if (!open || !path) {
			subtree = null;
			subtreeErr = null;
			return;
		}
		subtreeRetry;
		loading = true;
		subtreeErr = null;
		const ac = new AbortController();
		const p = path;
		void fetchTree(p, {
			depth: 64,
			branches: true,
			archived: true,
			activity: true,
			signal: ac.signal
		})
			.then((n) => {
				subtree = n;
			})
			.catch((e) => {
				subtree = null;
				subtreeErr = e instanceof Error ? e.message : 'Failed to load subtree';
			})
			.finally(() => {
				loading = false;
			});
		return () => ac.abort();
	});

	$effect(() => {
		path;
		open;
		if (open && path) {
			const s = segments(path);
			newSegment = s.length ? (s[s.length - 1] ?? '') : '';
		}
	});

	function confirm() {
		const seg = newSegment.trim();
		if (!seg || !path) return;
		actorPrompt.run(async () => {
			busy = true;
			try {
				await apiRename(path, seg);
				toast.success('Renamed');
				onConfirm(seg);
				open = false;
				onClose();
			} catch (e) {
				toast.error(e instanceof Error ? e.message : 'Rename failed');
			} finally {
				busy = false;
			}
		});
	}
</script>

<Dialog.Root
	bind:open
	onOpenChange={(v: boolean) => {
		if (!v) onClose();
	}}
>
	<Dialog.Content class="max-h-[85vh] overflow-y-auto border-border bg-card text-foreground sm:max-w-lg">
		<Dialog.Header>
			<Dialog.Title>Rename</Dialog.Title>
			<Dialog.Description class="text-foreground/70">
				Rename last segment of <span class="font-mono text-foreground">{path || '—'}</span>
			</Dialog.Description>
		</Dialog.Header>

		<div class="flex flex-col gap-3 py-2">
			<div>
				<label class="text-xs text-muted-foreground" for="rename-seg">New segment name</label>
				<Input
					id="rename-seg"
					bind:value={newSegment}
					class="mt-1 border-border bg-background font-mono text-sm"
					autocomplete="off"
				/>
			</div>

			{#if loading}
				<div class="flex items-center gap-2 text-sm text-muted-foreground">
					<Loader2 class="size-4 animate-spin" />
					Loading affected paths…
				</div>
			{:else if subtreeErr}
				<div class="flex flex-col gap-2 border border-destructive/40 bg-destructive/10 px-3 py-2 text-sm text-destructive">
					<p>{subtreeErr}</p>
					<Button
						variant="outline"
						size="sm"
						class="w-fit border-destructive/50 text-destructive hover:bg-destructive/15"
						onclick={() => subtreeRetry++}
					>
						<RefreshCw class="mr-1 size-3" />
						Retry
					</Button>
				</div>
			{:else if previewRowsAll.length > 0}
				<div>
					<p class="text-xs text-muted-foreground">
						This will change {previewRowsAll.length} path{previewRowsAll.length === 1 ? '' : 's'}:
					</p>
					<ul class="mt-2 max-h-40 overflow-y-auto thin-scrollbar border border-border/60 bg-background/60 p-2 font-mono text-[11px] text-foreground/80">
						{#each previewRows as r (r.from)}
							<li class="border-b border-border/40 py-1 last:border-0">
								<div class="truncate text-muted-foreground">{r.from}</div>
								<div class="truncate text-branch-active">→ {r.to}</div>
							</li>
						{/each}
					</ul>
					{#if previewExtra > 0}
						<p class="mt-1 text-[11px] text-muted-foreground">… and {previewExtra} more</p>
					{/if}
				</div>
			{/if}

			<div class="border border-primary/30 bg-primary/5 p-3 text-sm text-foreground">
				<p class="font-medium">Running agents</p>
				<p class="mt-1 text-xs leading-relaxed text-foreground/80">
					Agents still using the old path may fail on their next write after this rename.
				</p>
				{#if activeSessions.length > 0}
					<p class="mt-2 text-xs text-foreground/80">
						{activeSessions.length} session{activeSessions.length === 1 ? '' : 's'} with activity in the last 15 minutes:
					</p>
					<ul class="mt-1 space-y-1 text-xs text-foreground/80">
						{#each activeSessions as s (s.rel)}
							<li class="flex flex-wrap items-center gap-1">
								<span class="font-mono text-[11px] text-foreground">· {s.rel}</span>
								{#each s.branches as b (b)}
									<Badge variant="outline" class="text-[10px]">{b}</Badge>
								{/each}
							</li>
						{/each}
					</ul>
				{:else}
					<p class="mt-2 text-xs text-muted-foreground">No recent session writes detected under this path.</p>
				{/if}
			</div>
		</div>

		<Dialog.Footer>
			<Button variant="outline" onclick={() => (open = false)}>Cancel</Button>
			<Button
				disabled={busy || !newSegment.trim() || loading}
				onclick={() => void confirm()}
			>
				{#if busy}<Loader2 class="mr-1 size-3 animate-spin" />{/if}
				I understand — rename
			</Button>
		</Dialog.Footer>
	</Dialog.Content>
</Dialog.Root>
