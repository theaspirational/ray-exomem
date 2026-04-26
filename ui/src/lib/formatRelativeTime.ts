export function formatRelativeTime(iso: string | null | undefined): string {
	if (iso == null || iso === '') return '—';
	const t = new Date(iso).getTime();
	if (Number.isNaN(t)) return '—';
	const now = Date.now();
	const diffMs = t - now;
	const rtf = new Intl.RelativeTimeFormat(undefined, { numeric: 'auto' });
	const abs = Math.abs(diffMs);
	if (abs < 60_000) return rtf.format(Math.round(diffMs / 1000), 'second');
	if (abs < 3_600_000) return rtf.format(Math.round(diffMs / 60_000), 'minute');
	if (abs < 86_400_000) return rtf.format(Math.round(diffMs / 3_600_000), 'hour');
	if (abs < 604_800_000) return rtf.format(Math.round(diffMs / 86_400_000), 'day');
	if (abs < 2_592_000_000) return rtf.format(Math.round(diffMs / 604_800_000), 'week');
	if (abs < 31_536_000_000) return rtf.format(Math.round(diffMs / 2_592_000_000), 'month');
	return rtf.format(Math.round(diffMs / 31_536_000_000), 'year');
}
