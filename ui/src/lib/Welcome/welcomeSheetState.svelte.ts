let open = $state(false);

export const welcomeSheetState = {
	get open() {
		return open;
	},
	set open(v: boolean) {
		open = v;
	},
	openSheet() {
		open = true;
	},
	closeSheet() {
		open = false;
	}
};
