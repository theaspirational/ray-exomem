<script lang="ts">
	import { browser } from '$app/environment';
	import { Loader2 } from '@lucide/svelte';
	import { toast } from 'svelte-sonner';
	import { Button } from '$lib/components/ui/button/index.js';
	import * as Dialog from '$lib/components/ui/dialog/index.js';
	import { Input } from '$lib/components/ui/input/index.js';
	import { actorPrompt } from '$lib/actorPrompt.svelte';

	let draft = $state('');
	let busy = $state(false);
	/** Mirrors `actorPrompt.open` for reliable `bind:open` on the dialog. */
	let dialogOpen = $state(false);

	$effect(() => {
		dialogOpen = actorPrompt.open;
	});

	$effect(() => {
		actorPrompt.open;
		if (actorPrompt.open && browser) {
			draft = localStorage.getItem('ray-exomem-actor')?.trim() ?? '';
		}
	});

	async function save() {
		const v = draft.trim();
		if (!v) {
			toast.error('Enter an actor name (e.g. your handle or agent id).');
			return;
		}
		busy = true;
		try {
			localStorage.setItem('ray-exomem-actor', v);
			actorPrompt.commitSaved();
		} finally {
			busy = false;
		}
	}
</script>

<Dialog.Root
	bind:open={dialogOpen}
	onOpenChange={(v: boolean) => {
		if (!v) actorPrompt.cancel();
	}}
>
	<Dialog.Content class="border-zinc-700 bg-zinc-900 text-zinc-100 sm:max-w-md">
		<Dialog.Header>
			<Dialog.Title>Set actor identity</Dialog.Title>
			<Dialog.Description class="text-zinc-400">
				Writes are attributed to this name (sent as <span class="font-mono">X-Actor</span>). Set it once per
				browser profile.
			</Dialog.Description>
		</Dialog.Header>
		<div class="flex flex-col gap-2 py-2">
			<label class="text-xs text-zinc-500" for="actor-id">Actor</label>
			<Input
				id="actor-id"
				bind:value={draft}
				class="border-zinc-700 bg-zinc-950 font-mono text-sm"
				placeholder="e.g. alice or claude-code"
				autocomplete="username"
				onkeydown={(e) => {
					if (e.key === 'Enter') void save();
				}}
			/>
		</div>
		<Dialog.Footer>
			<Button type="button" variant="outline" class="border-zinc-600" onclick={() => actorPrompt.cancel()}>
				Cancel
			</Button>
			<Button type="button" disabled={busy || !draft.trim()} onclick={() => void save()}>
				{#if busy}<Loader2 class="mr-1 size-3 animate-spin" />{/if}
				Continue
			</Button>
		</Dialog.Footer>
	</Dialog.Content>
</Dialog.Root>
