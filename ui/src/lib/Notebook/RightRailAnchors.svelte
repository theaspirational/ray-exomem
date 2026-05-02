<script lang="ts">
	import { onMount } from 'svelte';
	import { browser } from '$app/environment';

	let { sections }: { sections: { id: string; label: string }[] } = $props();

	let active = $state('');
	let scrollRoot: Element | Window | null = null;

	function scrollToId(id: string) {
		if (!browser) return;
		active = id;
		document.getElementById(id)?.scrollIntoView({ behavior: 'smooth', block: 'start' });
	}

	function findScrollRoot(): Element | Window {
		const firstSection = document.getElementById(sections[0]?.id ?? '');
		let el = firstSection?.parentElement ?? null;
		while (el && el !== document.documentElement) {
			const overflowY = window.getComputedStyle(el).overflowY;
			if (
				/(auto|scroll|overlay)/.test(overflowY) &&
				el.scrollHeight > el.clientHeight
			) {
				return el;
			}
			el = el.parentElement;
		}
		return window;
	}

	function updateActive() {
		if (!browser || sections.length === 0) return;
		const root = scrollRoot ?? findScrollRoot();
		const top = root === window ? 0 : (root as Element).getBoundingClientRect().top;
		const y = top + 96;
		let current = sections[0].id;
		for (const s of sections) {
			const el = document.getElementById(s.id);
			if (el && el.getBoundingClientRect().top <= y) current = s.id;
		}
		active = current;
	}

	onMount(() => {
		active = sections[0]?.id ?? '';
		scrollRoot = findScrollRoot();
		updateActive();
		scrollRoot.addEventListener('scroll', updateActive, { passive: true });
		window.addEventListener('resize', updateActive, { passive: true });
		return () => {
			scrollRoot?.removeEventListener('scroll', updateActive);
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
