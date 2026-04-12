import { browser } from '$app/environment';

/** True when the user has set a non-empty `ray-exomem-actor` in localStorage. */
export function isActorIdentityConfigured(): boolean {
	if (!browser) return true;
	return Boolean(localStorage.getItem('ray-exomem-actor')?.trim());
}

class ActorPromptState {
	open = $state(false);
	/** Bumped after save so TopBar and others can re-read localStorage. */
	refreshSignal = $state(0);
	private pending: Array<() => void> = [];

	/**
	 * Runs the callback after the user has set an actor (or immediately if already configured).
	 * If the dialog is cancelled, pending callbacks are dropped.
	 */
	run(fn: () => void | Promise<void>): void {
		if (isActorIdentityConfigured()) {
			void Promise.resolve(fn());
			return;
		}
		this.pending.push(() => void Promise.resolve(fn()));
		this.open = true;
	}

	commitSaved(): void {
		this.refreshSignal++;
		this.open = false;
		const q = this.pending;
		this.pending = [];
		for (const f of q) f();
	}

	cancel(): void {
		this.open = false;
		this.pending = [];
	}
}

export const actorPrompt = new ActorPromptState();
