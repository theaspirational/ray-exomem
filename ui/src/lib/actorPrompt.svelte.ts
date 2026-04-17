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

	run(fn: () => void | Promise<void>): void {
		void Promise.resolve(fn());
	}

	commitSaved(): void {
		this.refreshSignal++;
		this.open = false;
	}

	cancel(): void {
		this.open = false;
	}
}

export const actorPrompt = new ActorPromptState();
