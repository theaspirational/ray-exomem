<script lang="ts">
	import { Loader2 } from '@lucide/svelte';
	import { toast } from 'svelte-sonner';
	import { Button } from '$lib/components/ui/button/index.js';
	import * as Dialog from '$lib/components/ui/dialog/index.js';
	import { Input } from '$lib/components/ui/input/index.js';
	import { actorPrompt } from '$lib/actorPrompt.svelte';
	import { apiAssertSessionLabel } from '$lib/exomem.svelte';

	let {
		open = $bindable(false),
		sessionPath = '',
		currentLabel = '',
		onClose
	}: {
		open?: boolean;
		sessionPath: string;
		currentLabel: string;
		onClose: () => void;
	} = $props();

	let value = $state('');
	let busy = $state(false);

	$effect(() => {
		sessionPath;
		currentLabel;
		open;
		if (open) value = currentLabel;
	});

	function submit() {
		const v = value.trim();
		if (!v || !sessionPath) return;
		actorPrompt.run(async () => {
			busy = true;
			try {
				await apiAssertSessionLabel(sessionPath, v);
				toast.success('Label updated');
				open = false;
				onClose();
			} catch (e) {
				toast.error(e instanceof Error ? e.message : 'Update failed');
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
	<Dialog.Content class="border-zinc-700 bg-zinc-900 text-zinc-100 sm:max-w-md">
		<Dialog.Header>
			<Dialog.Title>Rename session label</Dialog.Title>
			<Dialog.Description class="font-mono text-xs text-zinc-500">
				{sessionPath}
			</Dialog.Description>
		</Dialog.Header>
		<form
			class="flex flex-col gap-3 py-2"
			onsubmit={(e) => {
				e.preventDefault();
				void submit();
			}}
		>
			<div>
				<label class="text-xs text-zinc-500" for="sess-label">Display label</label>
				<Input
					id="sess-label"
					bind:value
					class="mt-1 border-zinc-700 bg-zinc-950 text-sm"
					autocomplete="off"
				/>
			</div>
			<Dialog.Footer>
				<Button type="button" variant="outline" class="border-zinc-600" onclick={() => (open = false)}>
					Cancel
				</Button>
				<Button type="submit" disabled={busy || !value.trim()}>
					{#if busy}<Loader2 class="mr-1 size-3 animate-spin" />{/if}
					Save
				</Button>
			</Dialog.Footer>
		</form>
	</Dialog.Content>
</Dialog.Root>
