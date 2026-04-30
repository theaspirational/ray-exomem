/**
 * Shared modal + drawer refresh coordination for tree context menu and command palette.
 */
const LAST_TOUCHED_KEY = 'ray-exomem:last-touched-tree-path';

function readLastTouched(): string {
	if (typeof localStorage === 'undefined') return '';
	try {
		return localStorage.getItem(LAST_TOUCHED_KEY) ?? '';
	} catch {
		return '';
	}
}

class TreeModalsState {
	renameOpen = $state(false);
	renamePath = $state('');

	sessionLabelOpen = $state(false);
	sessionLabelPath = $state('');
	sessionLabelCurrent = $state('');

	initOpen = $state(false);
	initPath = $state('');
	folderOpen = $state(false);
	folderPathField = $state('');
	exomOpen = $state(false);
	exomPathField = $state('');
	sessionOpen = $state(false);
	sessionProjectPath = $state('');
	sessionLabelField = $state('adhoc');

	deleteOpen = $state(false);
	deletePath = $state('');
	deleteKind = $state<'folder' | 'exom'>('folder');

	/** Incremented after mutations so `TreeDrawer` can reload. */
	refreshTree = $state(0);

	/** Last tree path the user touched (navigated to or acted on). Persists across reloads. */
	lastTouchedPath = $state(readLastTouched());

	setLastTouched(path: string) {
		const trimmed = path.trim();
		if (!trimmed || trimmed === this.lastTouchedPath) return;
		this.lastTouchedPath = trimmed;
		if (typeof localStorage === 'undefined') return;
		try {
			localStorage.setItem(LAST_TOUCHED_KEY, trimmed);
		} catch {
			// ignore
		}
	}

	openRename(path: string) {
		this.renamePath = path;
		this.renameOpen = true;
	}

	openSessionLabel(sessionPath: string, currentLabel: string) {
		this.sessionLabelPath = sessionPath;
		this.sessionLabelCurrent = currentLabel;
		this.sessionLabelOpen = true;
	}

	openInit(path: string) {
		this.initPath = path;
		this.initOpen = true;
	}

	openNewFolder(path: string) {
		this.folderPathField = path;
		this.folderOpen = true;
	}

	openNewExom(suggestedPath: string) {
		this.exomPathField = suggestedPath;
		this.exomOpen = true;
	}

	openNewSession(projectFolderPath: string) {
		this.sessionProjectPath = projectFolderPath;
		this.sessionLabelField = 'adhoc';
		this.sessionOpen = true;
	}

	openDelete(path: string, kind: 'folder' | 'exom') {
		this.deletePath = path;
		this.deleteKind = kind;
		this.deleteOpen = true;
	}

	bumpTree() {
		this.refreshTree++;
	}
}

export const treeModals = new TreeModalsState();
