let open = $state(false);

export const commandPaletteState = {
	get open() {
		return open;
	},
	set open(v: boolean) {
		open = v;
	},
	show() {
		open = true;
	}
};
