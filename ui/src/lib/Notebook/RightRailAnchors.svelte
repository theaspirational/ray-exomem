<script lang="ts">
	import { onMount } from 'svelte';
	import { browser } from '$app/environment';

	let { sections }: { sections: { id: string; label: string }[] } = $props();

	let active = $state('');

	function scrollToId(id: string) {
		if (!browser) return;
		document.getElementById(id)?.scrollIntoView({ behavior: 'smooth', block: 'start' });
	}

	function updateActive() {
		if (!browser || sections.length === 0) return;
		const y = window.scrollY + 96;
		let current = sections[0].id;
		for (const s of sections) {
			const el = document.getElementById(s.id);
			if (el && el.offsetTop <= y) current = s.id;
		}
		active = current;
	}

	onMount(() => {
		active = sections[0]?.id ?? '';
		updateActive();
		window.addEventListener('scroll', updateActive, { passive: true });
		window.addEventListener('resize', updateActive, { passive: true });
		return () => {
			window.removeEventListener('scroll', updateActive);
			window.removeEventListener('resize', updateActive);
		};
	});
</script>

<nav class="flex flex-col gap-1 font-mono text-[12px] text-muted-foreground" aria-label="On this page">
	{#each sections as s (s.id)}
		<a
			href="#{s.id}"
			onclick={(e) => {
				e.preventDefault();
				scrollToId(s.id);
			}}
			class="rounded-sm px-1 py-0.5 transition-colors hover:text-foreground {active === s.id
				? 'text-primary'
				: ''}"
		>
			{s.label}
		</a>
	{/each}
</nav>
