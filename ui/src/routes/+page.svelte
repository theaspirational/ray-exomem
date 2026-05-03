<script lang="ts">
	import { onMount } from 'svelte';
	import { browser } from '$app/environment';
	import { base } from '$app/paths';
	import { Button } from '$lib/components/ui/button/index.js';
	import { getExomemBaseUrl } from '$lib/exomem.svelte';
	import FeaturedCards from '$lib/Welcome/FeaturedCards.svelte';
	import LatestChanges from '$lib/Welcome/LatestChanges.svelte';
	import StatsStrip from '$lib/Welcome/StatsStrip.svelte';
	import { welcomeSheetState } from '$lib/Welcome/welcomeSheetState.svelte';

	interface WelcomeSummary {
		totals: {
			facts: number;
			exoms: number;
			branches: number;
			last_change: { tx_time: string; actor: string; note?: string | null } | null;
		};
		featured: {
			exom: string;
			entity: string;
			name: string;
			type: string;
			summary: string | null;
			docs_url: string | null;
			fact_count: number;
			last_tx_time: string;
		}[];
		latest: {
			exom: string;
			tx_id: number;
			tx_time: string;
			actor: string;
			action: string;
			note: string | null;
			branch_id: string;
			refs: string[];
		}[];
	}

	let summary = $state<WelcomeSummary | null>(null);
	let loadError = $state<string | null>(null);
	let loading = $state(true);
	let browseHref = $state(`${base}/tree/`);

	onMount(() => {
		if (browser) {
			const last = localStorage.getItem('ray-exomem:last-exom')?.trim();
			browseHref = last
				? `${base}/tree/${last.split('/').map(encodeURIComponent).join('/')}`
				: `${base}/tree/`;
		}
		void loadSummary();
	});

	async function loadSummary() {
		loading = true;
		loadError = null;
		try {
			const r = await fetch(`${getExomemBaseUrl()}/api/welcome/summary`, {
				credentials: 'include'
			});
			if (!r.ok) {
				loadError = r.status === 401 ? 'Sign in required' : 'Could not load welcome';
				summary = null;
				return;
			}
			summary = (await r.json()) as WelcomeSummary;
		} catch {
			loadError = 'Could not load welcome';
			summary = null;
		} finally {
			loading = false;
		}
	}
</script>

<svelte:head>
	<title>Welcome — Ray Exomem</title>
</svelte:head>

<div class="min-h-full bg-background p-4 text-foreground sm:p-6 lg:p-8">
	<div class="mx-auto max-w-4xl space-y-10">
		<section class="space-y-4">
			<p class="font-serif text-xl leading-relaxed text-foreground sm:text-2xl">
				A shared brain, written by your team and the agents they connect.
			</p>
			<StatsStrip
				facts={summary?.totals.facts ?? 0}
				exoms={summary?.totals.exoms ?? 0}
				branches={summary?.totals.branches ?? 0}
				lastChange={summary?.totals.last_change ?? null}
				{loading}
			/>
			<div class="flex flex-wrap items-center gap-3">
				<Button
					variant="default"
					size="default"
					class="font-sans"
					onclick={() => welcomeSheetState.openSheet()}
				>
					Connect an agent
				</Button>
				<Button variant="outline" size="default" class="font-sans" href={browseHref}>
					Browse memory →
				</Button>
			</div>
		</section>

		{#if loadError}
			<p class="font-sans text-sm text-foreground/70">{loadError}</p>
		{:else if summary}
			{#if summary.featured.length > 0}
				<FeaturedCards items={summary.featured} />
			{/if}

			{#if summary.latest.length > 0}
				<LatestChanges items={summary.latest} />
			{/if}
		{/if}
	</div>
</div>
