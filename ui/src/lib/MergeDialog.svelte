<script lang="ts">
	import { Loader2, X } from '@lucide/svelte';

	import { Button } from '$lib/components/ui/button';
	import { actorPrompt } from '$lib/actorPrompt.svelte';
	import { fetchExomemStatus, mergeBranch } from '$lib/exomem.svelte';

	interface Props {
		sourceBranch: string;
		exom: string;
		onClose: () => void;
	}

	let { sourceBranch, exom, onClose }: Props = $props();

	let policy = $state<'last-writer-wins' | 'keep-target' | 'manual'>('manual');
	let busy = $state(false);
	let resultMessage = $state<string | null>(null);
	let errorMessage = $state<string | null>(null);
	let targetLabel = $state<string>('');

	$effect(() => {
		void (async () => {
			try {
				const s = await fetchExomemStatus(exom);
				targetLabel = s.current_branch ?? 'main';
			} catch {
				targetLabel = 'main';
			}
		})();
	});

	function runMerge() {
		actorPrompt.run(async () => {
			busy = true;
			resultMessage = null;
			errorMessage = null;
			try {
				const r = await mergeBranch(sourceBranch, policy, exom);
				if (r.conflicts.length > 0) {
					resultMessage = `Merge completed with ${r.conflicts.length} conflict(s) (manual policy may leave conflicts unresolved). Tx ${r.tx_id}.`;
				} else {
					resultMessage = `Merged ${r.added.length} fact(s). Tx ${r.tx_id}.`;
				}
			} catch (e) {
				errorMessage = e instanceof Error ? e.message : String(e);
			} finally {
				busy = false;
			}
		});
	}
</script>

<div
	class="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4"
	role="dialog"
	aria-modal="true"
	aria-labelledby="merge-title"
>
	<div class="w-full max-w-md rounded-lg border border-border bg-background p-5 shadow-lg">
		<div class="flex items-start justify-between gap-2">
			<div>
				<h2 id="merge-title" class="text-lg font-semibold">Merge branch</h2>
				<p class="mt-1 text-sm text-muted-foreground">
					Merge <span class="font-mono text-foreground">{sourceBranch}</span> into the current branch
					<span class="font-mono text-foreground">({targetLabel})</span>.
				</p>
			</div>
			<button type="button" class="rounded p-1 text-muted-foreground hover:bg-muted" onclick={onClose} aria-label="Close">
				<X class="size-4" />
			</button>
		</div>

		<div class="mt-4 space-y-2">
			<p class="text-xs font-medium text-muted-foreground">Policy</p>
			<div class="flex flex-col gap-2 text-sm">
				<label class="flex cursor-pointer items-center gap-2">
					<input type="radio" bind:group={policy} value="manual" />
					Manual — report conflicts, minimal auto changes
				</label>
				<label class="flex cursor-pointer items-center gap-2">
					<input type="radio" bind:group={policy} value="last-writer-wins" />
					Last writer wins
				</label>
				<label class="flex cursor-pointer items-center gap-2">
					<input type="radio" bind:group={policy} value="keep-target" />
					Keep target
				</label>
			</div>
		</div>

		{#if errorMessage}
			<p class="mt-3 rounded border border-destructive/30 bg-destructive/10 px-2 py-1.5 text-sm text-destructive">{errorMessage}</p>
		{/if}
		{#if resultMessage}
			<p class="mt-3 rounded border border-primary/30 bg-primary/5 px-2 py-1.5 text-sm text-foreground">{resultMessage}</p>
		{/if}

		<div class="mt-6 flex justify-end gap-2">
			<Button variant="outline" onclick={onClose}>Close</Button>
			<Button disabled={busy} onclick={() => void runMerge()} class="gap-1.5">
				{#if busy}
					<Loader2 class="size-4 animate-spin" />
				{/if}
				Run merge
			</Button>
		</div>
	</div>
</div>
