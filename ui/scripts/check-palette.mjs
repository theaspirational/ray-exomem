#!/usr/bin/env node
// Fails if any legacy Tailwind palette colors leak into the codebase.
// All colors should come from semantic tokens defined in src/app.css.

import { execSync } from 'node:child_process';

const BANNED_PREFIXES = [
	'zinc',
	'gray',
	'slate',
	'neutral',
	'stone',
	'red',
	'orange',
	'amber',
	'yellow',
	'lime',
	'green',
	'emerald',
	'teal',
	'cyan',
	'sky',
	'blue',
	'indigo',
	'violet',
	'purple',
	'fuchsia',
	'pink',
	'rose'
];

const pattern = `(${BANNED_PREFIXES.join('|')})-[0-9]`;

let out = '';
try {
	out = execSync(
		`grep -rEnH "${pattern}" src --include='*.svelte' --include='*.ts' --include='*.css' --include='*.js' || true`,
		{ encoding: 'utf8' }
	);
} catch (e) {
	console.error('palette-check: grep failed', e);
	process.exit(2);
}

if (out.trim().length === 0) {
	console.log('palette-check: ok — no legacy Tailwind palette colors found.');
	process.exit(0);
}

console.error('palette-check: FAILED — legacy Tailwind palette colors detected.');
console.error('Use semantic tokens from src/app.css (foreground, background, card, primary, destructive, ...) instead.');
console.error('');
console.error(out);
process.exit(1);
