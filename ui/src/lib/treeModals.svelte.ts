/**
 * Shared modal + drawer refresh coordination for tree context menu and command palette.
 */
class TreeModalsState {
	renameOpen = $state(false);
	renamePath = $state('');

	sessionLabelOpen = $state(false);
	sessionLabelPath = $state('');
	sessionLabelCurrent = $state('');

	initOpen = $state(false);
	initPath = $state('');
	exomOpen = $state(false);
	exomPathField = $state('');
	sessionOpen = $state(false);
	sessionProjectPath = $state('');
	sessionLabelField = $state('adhoc');

	/** Incremented after mutations so `TreeDrawer` can reload. */
	refreshTree = $state(0);

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

	openNewExom(suggestedPath: string) {
		this.exomPathField = suggestedPath;
		this.exomOpen = true;
	}

	openNewSession(projectFolderPath: string) {
		this.sessionProjectPath = projectFolderPath;
		this.sessionLabelField = 'adhoc';
		this.sessionOpen = true;
	}

	bumpTree() {
		this.refreshTree++;
	}
}

export const treeModals = new TreeModalsState();
