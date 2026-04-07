import { browser } from '$app/environment';
import { env } from '$env/dynamic/public';

import type {
	ExomemClusterSummary,
	ExomemGraphResponse,
	ExomemLoggedEvent,
	ExomemSchemaResponse,
	ExomemStatus,
	FactEntry,
	ExomEntry,
	RuleEntry
} from '$lib/types';

const DEFAULT_BASE_URL = 'http://127.0.0.1:9780';

/** Default KB name; must match server `DEFAULT_EXOM`. */
export const DEFAULT_EXOM = 'main';

/** Prevents the UI from staying on “Loading…” forever if TCP hangs or the daemon is unreachable. */
const DEFAULT_FETCH_TIMEOUT_MS = 60_000;

/**
 * AbortSignal for fetch timeout. Call `clear()` after fetch settles so the timer does not fire.
 * Browsers often reject aborted fetch with `TypeError: Failed to fetch` (not `AbortError`) — use
 * `signal.aborted` in catch, not only `e.name === 'AbortError'`.
 */
function signalWithTimeout(ms: number, outer?: AbortSignal): {
	signal: AbortSignal;
	clear: () => void;
} {
	const c = new AbortController();
	const t = setTimeout(() => c.abort(), ms);
	const clear = () => clearTimeout(t);
	const onOuterAbort = () => {
		clear();
		c.abort();
	};
	if (outer) {
		if (outer.aborted) {
			clear();
			c.abort();
		} else {
			outer.addEventListener('abort', onOuterAbort, { once: true });
		}
	}
	return { signal: c.signal, clear };
}

function fetchTimedOutMessage(): string {
	return `Request timed out after ${DEFAULT_FETCH_TIMEOUT_MS / 1000}s. Run ray-exomem daemon, then reload this page.`;
}

function normalizeBaseUrl(baseUrl: string): string {
	const trimmed = baseUrl.trim().replace(/\/+$/, '');
	return trimmed.endsWith('/ray-exomem') ? trimmed : `${trimmed}/ray-exomem`;
}

export function getExomemBaseUrl(): string {
	const configured = env.PUBLIC_TEIDE_EXOMEM_BASE_URL?.trim();
	if (configured) return normalizeBaseUrl(configured);

	if (browser) {
		const { origin, port } = window.location;
		// In the Vite dev server, the UI runs on 5173 and the daemon still lives on 9780.
		// When the UI is served from the daemon itself, use the current origin so LAN access
		// keeps working on phones/tablets and other machines.
		if (port !== '5173') return normalizeBaseUrl(origin);
	}

	return normalizeBaseUrl(DEFAULT_BASE_URL);
}

function endpoint(path: string): string {
	return `${getExomemBaseUrl()}/${path.replace(/^\/+/, '')}`;
}

export class ExomemLiveState {
	status = $state<'idle' | 'connecting' | 'open'>('idle');
	events = $state<ExomemLoggedEvent[]>([]);
	lastEvent = $state<ExomemLoggedEvent | null>(null);

	#source: EventSource | null = null;

	connect() {
		if (typeof window === 'undefined' || this.#source) return;

		this.status = 'connecting';
		const source = new EventSource(endpoint('events'));
		this.#source = source;

		source.addEventListener('open', () => {
			this.status = 'open';
		});

		const handleEvent = (raw: MessageEvent<string>) => {
			const event = JSON.parse(raw.data) as ExomemLoggedEvent;
			this.lastEvent = event;
			this.events = [event, ...this.events].slice(0, 80);
		};

		for (const type of ['query', 'assert', 'retract', 'evaluate', 'load']) {
			source.addEventListener(type, handleEvent as EventListener);
		}

		source.onerror = () => {
			this.disconnect();
		};
	}

	disconnect() {
		this.#source?.close();
		this.#source = null;
		this.status = 'idle';
	}
}

// ---------------------------------------------------------------------------
// HTTP helpers
// ---------------------------------------------------------------------------

async function readJson<T>(path: string, init?: RequestInit): Promise<T> {
	let res: Response;
	const { signal, clear } = signalWithTimeout(DEFAULT_FETCH_TIMEOUT_MS, init?.signal);
	try {
		res = await fetch(endpoint(path), { ...init, signal });
		clear();
	} catch (e) {
		clear();
		if (signal.aborted) {
			if (init?.signal?.aborted) {
				throw e instanceof Error ? e : new Error('Aborted');
			}
			throw new Error(fetchTimedOutMessage());
		}
		const msg = e instanceof Error ? e.message : String(e);
		const base = getExomemBaseUrl();
		if (msg === 'Failed to fetch' || msg.includes('NetworkError')) {
			throw new Error(
				`${msg} at ${base}. Start the server with: ray-exomem daemon`
			);
		}
		throw e instanceof Error ? e : new Error(msg);
	}
	if (!res.ok) {
		throw new Error(`Request failed: ${res.status} ${res.statusText}`);
	}
	return res.json();
}

async function postAction<T>(path: string, body?: unknown): Promise<T> {
	let res: Response;
	const { signal, clear } = signalWithTimeout(DEFAULT_FETCH_TIMEOUT_MS);
	try {
		res = await fetch(endpoint(path), {
			method: 'POST',
			headers: { 'Content-Type': 'application/json' },
			body: body !== undefined ? JSON.stringify(body) : undefined,
			signal
		});
		clear();
	} catch (e) {
		clear();
		if (signal.aborted) throw new Error(fetchTimedOutMessage());
		const msg = e instanceof Error ? e.message : String(e);
		throw new Error(msg);
	}
	if (!res.ok) {
		throw new Error(`Action failed: ${res.status} ${res.statusText}`);
	}
	return res.json();
}

async function postText<T>(path: string, body: string): Promise<T> {
	const { signal, clear } = signalWithTimeout(DEFAULT_FETCH_TIMEOUT_MS);
	let res: Response;
	try {
		res = await fetch(endpoint(path), {
			method: 'POST',
			headers: { 'Content-Type': 'text/plain' },
			body,
			signal
		});
		clear();
	} catch (e) {
		clear();
		if (signal.aborted) throw new Error(fetchTimedOutMessage());
		throw e instanceof Error ? e : new Error(String(e));
	}
	if (!res.ok) throw new Error(`Action failed: ${res.status} ${res.statusText}`);
	return res.json();
}

// ---------------------------------------------------------------------------
// Status / Schema / Clusters / Logs
// ---------------------------------------------------------------------------

export function fetchExomemStatus(exom = DEFAULT_EXOM): Promise<ExomemStatus> {
	return readJson<ExomemStatus>(`api/status?exom=${encodeURIComponent(exom)}`);
}

export function fetchExomemSchema(
	exom = DEFAULT_EXOM,
	signal?: AbortSignal
): Promise<ExomemSchemaResponse> {
	return readJson<ExomemSchemaResponse>(
		`api/schema?include_samples=true&sample_limit=2&exom=${encodeURIComponent(exom)}`,
		signal ? { signal } : undefined
	);
}

export function fetchExomemGraph(limit = 500, exom = DEFAULT_EXOM): Promise<ExomemGraphResponse> {
	return readJson<ExomemGraphResponse>(
		`api/graph?limit=${limit}&cluster=true&exom=${encodeURIComponent(exom)}`
	);
}

export interface RelationGraphResponse {
	nodes: Array<{ id: string; label: string; degree: number }>;
	edges: Array<{
		source: string;
		target: string;
		label: string;
		predicate: string;
		kind: 'base' | 'derived';
	}>;
	summary: { node_count: number; edge_count: number };
}

export function fetchRelationGraph(exom = DEFAULT_EXOM): Promise<RelationGraphResponse> {
	return readJson<RelationGraphResponse>(
		`api/relation-graph?exom=${encodeURIComponent(exom)}`
	);
}

/**
 * Fetch all facts involving a given entity by querying all relations for tuples
 * that mention the entity in any position.
 */
export async function fetchEntityFacts(
	entity: string,
	exom = DEFAULT_EXOM
): Promise<FactEntry[]> {
	const schema = await readJson<ExomemSchemaResponse>(
		`api/schema?include_samples=true&sample_limit=10000&exom=${encodeURIComponent(exom)}`
	);
	const facts: FactEntry[] = [];
	for (const rel of schema.relations) {
		if (!rel.sample_tuples) continue;
		for (const tuple of rel.sample_tuples) {
			const terms = tuple.map(String);
			if (terms.some((t) => t === entity)) {
				facts.push({
					predicate: rel.name,
					terms,
					kind: rel.kind,
					confidence: null,
					source: null
				});
			}
		}
	}
	return facts;
}

export async function fetchExomemClusters(exom = DEFAULT_EXOM): Promise<ExomemClusterSummary[]> {
	const payload = await readJson<{ clusters: ExomemClusterSummary[] }>(
		`api/clusters?kind=all&limit=64&exom=${encodeURIComponent(exom)}`
	);
	return payload.clusters;
}

export async function fetchExomemLogs(exom = DEFAULT_EXOM): Promise<ExomemLoggedEvent[]> {
	const payload = await readJson<{ events: ExomemLoggedEvent[] }>(
		`api/logs?limit=24&type=all&exom=${encodeURIComponent(exom)}`
	);
	return payload.events;
}

export async function fetchExoms(): Promise<ExomEntry[]> {
	const payload = await readJson<{ exoms: ExomEntry[] }>('api/exoms');
	return payload.exoms;
}

// ---------------------------------------------------------------------------
// Database Actions
// ---------------------------------------------------------------------------

export function clearDatabase(
	exom = DEFAULT_EXOM
): Promise<{ ok: boolean; tuples_removed: number }> {
	return postAction(`api/actions/clear?exom=${encodeURIComponent(exom)}`);
}

export function retractFact(
	predicate: string,
	terms: unknown[],
	exom = DEFAULT_EXOM
): Promise<{ ok: boolean; retracted: number }> {
	return postAction('api/actions/retract', { predicate, terms, exom });
}

export function dropRelation(
	relation: string,
	exom = DEFAULT_EXOM
): Promise<{ ok: boolean; relation: string; tuples_removed: number }> {
	return postAction('api/actions/drop-relation', { relation, exom });
}

export function triggerEvaluate(exom = DEFAULT_EXOM): Promise<{
	ok: boolean;
	new_derivations: number;
	duration_ms: number;
}> {
	return postAction(`api/actions/evaluate?exom=${encodeURIComponent(exom)}`);
}

export async function exportBackup(exom = DEFAULT_EXOM): Promise<void> {
	const url = endpoint(`api/actions/export?exom=${encodeURIComponent(exom)}`);
	const { signal, clear } = signalWithTimeout(DEFAULT_FETCH_TIMEOUT_MS);
	let res: Response;
	try {
		res = await fetch(url, { signal });
		clear();
	} catch (e) {
		clear();
		if (signal.aborted) throw new Error(fetchTimedOutMessage());
		throw e instanceof Error ? e : new Error(String(e));
	}
	if (!res.ok) throw new Error(`Export failed: ${res.status}`);
	const text = await res.text();
	const blob = new Blob([text], { type: 'text/plain' });
	const a = document.createElement('a');
	a.href = URL.createObjectURL(blob);
	a.download = `exomem-${exom}-${new Date().toISOString().slice(0, 10)}.ray`;
	a.click();
	URL.revokeObjectURL(a.href);
}

export async function exportBackupText(exom = DEFAULT_EXOM, signal?: AbortSignal): Promise<string> {
	const url = endpoint(`api/actions/export?exom=${encodeURIComponent(exom)}`);
	const { signal: merged, clear } = signalWithTimeout(DEFAULT_FETCH_TIMEOUT_MS, signal);
	let res: Response;
	try {
		res = await fetch(url, { signal: merged });
		clear();
	} catch (e) {
		clear();
		if (merged.aborted) {
			if (signal?.aborted) throw e instanceof Error ? e : new Error('Aborted');
			throw new Error(fetchTimedOutMessage());
		}
		throw e instanceof Error ? e : new Error(String(e));
	}
	if (!res.ok) throw new Error(`Export failed: ${res.status}`);
	return res.text();
}

export function importBackup(
	source: string,
	exom = DEFAULT_EXOM
): Promise<{ ok: boolean; facts_added: number; rules_added: number; total_tuples: number }> {
	return postText(`api/actions/import?exom=${encodeURIComponent(exom)}`, source);
}

// ---------------------------------------------------------------------------
// Exom management
// ---------------------------------------------------------------------------

export function createExom(
	name: string,
	description: string,
	copyFrom?: string
): Promise<{ ok: boolean; name: string }> {
	return postAction('api/exoms', { name, description, copy_from: copyFrom });
}

export function manageExom(
	name: string,
	action: 'rename' | 'update_description' | 'archive' | 'unarchive' | 'delete',
	extra?: { new_name?: string; description?: string; confirm?: boolean }
): Promise<{ ok: boolean }> {
	return postAction(`api/exoms/${encodeURIComponent(name)}/manage`, {
		action,
		...extra
	});
}

export function mergeExoms(
	sources: string[],
	target: string,
	description: string,
	strategy: 'union' | 'prefer_left' | 'prefer_right' | 'flag_conflicts' = 'union',
	confidenceMerge: 'max' | 'min' | 'avg' = 'max'
): Promise<{ ok: boolean; name: string }> {
	return postAction('api/exoms/merge', {
		sources,
		target,
		description,
		strategy,
		confidence_merge: confidenceMerge
	});
}

// ---------------------------------------------------------------------------
// Provenance & Explain
// ---------------------------------------------------------------------------

export interface ProvenanceNode {
	id: string;
	predicate: string;
	terms: unknown[];
	kind: 'base' | 'derived';
	rule_index?: number;
	rule_head?: string;
	sources?: Array<{ id: string; predicate: string; terms: unknown[] }>;
	source?: string | null;
	confidence?: number | null;
	asserted_at?: number | null;
}

export interface ProvenanceEdge {
	source: string;
	target: string;
	rule_index: number;
	rule_head: string;
	confidence: number | null;
}

export interface ProvenanceResponse {
	derivations: ProvenanceNode[];
	base_facts: ProvenanceNode[];
	edges: ProvenanceEdge[];
	timeline: Array<Record<string, unknown>>;
	summary: {
		derived_count: number;
		base_count: number;
		edge_count: number;
		event_count: number;
	};
}

export interface ProofTreeNode {
	id: string;
	predicate: string;
	terms: unknown[];
	kind: 'base' | 'derived';
	truncated?: boolean;
	derivations?: Array<{
		rule_index: number;
		rule_head: string;
		sources: ProofTreeNode[];
	}>;
	source?: string | null;
	confidence?: number | null;
	asserted_at?: number | null;
}

export interface ExplainResponse {
	predicate: string;
	terms: string[];
	tree: ProofTreeNode;
	meta: { source: string | null; confidence: number | null; asserted_at: number | null } | null;
}

export function fetchProvenance(exom = DEFAULT_EXOM): Promise<ProvenanceResponse> {
	return readJson<ProvenanceResponse>(
		`api/provenance?exom=${encodeURIComponent(exom)}`
	);
}

export function fetchExplain(
	predicate: string,
	terms: string[],
	depth = 10,
	exom = DEFAULT_EXOM
): Promise<ExplainResponse> {
	return readJson<ExplainResponse>(
		`api/explain?predicate=${encodeURIComponent(predicate)}&terms=${encodeURIComponent(terms.join(','))}&depth=${depth}&exom=${encodeURIComponent(exom)}`
	);
}

// ---------------------------------------------------------------------------
// Fact CRUD — uses import/retract endpoints
// ---------------------------------------------------------------------------

/**
 * Format a fact as a Rayfall s-expression for import.
 */
export function formatFact(predicate: string, terms: string[]): string {
	const value = terms.map((t) => `"${t.replace(/"/g, '\\"')}"`).join(' ');
	return `(assert-fact "${predicate.replace(/"/g, '\\"')}" ${value})`;
}

export function assertFact(
	predicate: string,
	terms: string[],
	options?: {
		validFrom?: string;
		validTo?: string;
		confidence?: number;
		source?: string;
	},
	exom = DEFAULT_EXOM
): Promise<{ ok: boolean; fact_id?: string }> {
	return postAction(`api/actions/assert-fact?exom=${encodeURIComponent(exom)}`, {
		predicate,
		value: terms.join(', '),
		confidence: options?.confidence ?? 1.0,
		provenance: options?.source ?? 'ui',
		valid_from: options?.validFrom,
		valid_to: options?.validTo
	});
}

export async function updateFact(
	oldFact: Pick<FactEntry, 'predicate' | 'terms'>,
	newFact: {
		predicate: string;
		terms: string[];
		validFrom?: string;
		validTo?: string;
	},
	exom = DEFAULT_EXOM
): Promise<{ ok: boolean; facts_added?: number; rules_added?: number; total_tuples?: number }> {
	await retractFact(oldFact.predicate, oldFact.terms, exom);
	return assertFact(newFact.predicate, newFact.terms, { validFrom: newFact.validFrom, validTo: newFact.validTo }, exom);
}

/**
 * Add a rule by importing it as Rayfall syntax.
 */
export function addRule(
	ruleSource: string,
	exom = DEFAULT_EXOM
): Promise<{ ok: boolean; facts_added: number; rules_added: number; total_tuples: number }> {
	return importBackup(ruleSource, exom);
}

/**
 * Fetch all facts for a given relation by requesting full samples from the schema endpoint.
 */
export async function fetchRelationFacts(
	relation: string,
	exom = DEFAULT_EXOM
): Promise<ExomemSchemaResponse> {
	return readJson<ExomemSchemaResponse>(
		`api/schema?include_samples=true&sample_limit=1000&relation=${encodeURIComponent(relation)}&exom=${encodeURIComponent(exom)}`
	);
}

/**
 * Build a flat list of FactEntry from the schema response.
 */
export function schemaToFacts(schema: ExomemSchemaResponse): FactEntry[] {
	const facts: FactEntry[] = [];
	for (const rel of schema.relations) {
		if (rel.sample_tuples) {
			for (const tuple of rel.sample_tuples) {
				// Last element may be a validity object { valid_from, valid_to }
				const last = tuple[tuple.length - 1];
				let validFrom: string | null = null;
				let validTo: string | null = null;
				let terms: string[];
				let branchRole: FactEntry['branchRole'];
				let branchOrigin: string | null = null;
				if (last && typeof last === 'object' && 'valid_from' in (last as Record<string, unknown>)) {
					const validity = last as {
						valid_from?: string;
						valid_to?: string | null;
						branch_role?: string;
						branch_origin?: string;
					};
					validFrom = validity.valid_from ?? null;
					validTo = validity.valid_to ?? null;
					branchOrigin = validity.branch_origin ?? null;
					const br = validity.branch_role;
					if (br === 'local' || br === 'inherited' || br === 'override') {
						branchRole = br;
					}
					terms = tuple.slice(0, -1).map(String);
				} else {
					terms = tuple.map(String);
				}
				facts.push({
					predicate: rel.name,
					terms,
					kind: rel.kind,
					confidence: null,
					source: null,
					validFrom,
					validTo,
					branchRole,
					branchOrigin
				});
			}
		}
	}
	return facts;
}

/**
 * Parse the exported Rayfall text to extract rules as structured entries.
 */
export function parseRulesFromExport(text: string): RuleEntry[] {
	const rules: RuleEntry[] = [];
	const lines = text.split('\n');
	let ruleIndex = 0;

	for (const line of lines) {
		const trimmed = line.trim();
		if (!trimmed || trimmed.startsWith(';')) continue;

		// Match: (rule exom (head ...) (body ...)) — exom name is mandatory
		const ruleMatch = trimmed.match(/^\(rule\s+\w+\s+\((\w+)\s+([^)]*)\)\s+(.*)\)$/);
		if (ruleMatch) {
			const headPredicate = ruleMatch[1];
			const bodyRaw = ruleMatch[3];
			// Split body into atoms by matching parenthesized expressions
			const bodyAtoms = (bodyRaw.match(/\([^)]+\)/g) || []).map((a) => a.trim());

			rules.push({
				index: ruleIndex++,
				raw: trimmed,
				head_predicate: headPredicate,
				body_atoms: bodyAtoms,
				uses_negation: bodyAtoms.some((a) => a.startsWith('(!')),
				uses_temporal: false
			});
		}
	}
	return rules;
}

/**
 * Parse facts from exported Rayfall text.
 */
export function parseFactsFromExport(text: string): FactEntry[] {
	const facts: FactEntry[] = [];
	const lines = text.split('\n');

	for (const line of lines) {
		const trimmed = line.trim();
		if (!trimmed || trimmed.startsWith(';;') && !trimmed.includes('assert-fact')) continue;

		// Match: (assert-fact ...) possibly followed by ;; @valid[from, to]
		const factMatch = trimmed.match(/^\(assert-fact\s+(.*)\)/);
		if (factMatch) {
			const argsStr = factMatch[1];
			// Extract all double-quoted strings
			const strings: string[] = [];
			const re = /"((?:[^"\\]|\\.)*)"/g;
			let m;
			while ((m = re.exec(argsStr)) !== null) {
				strings.push(m[1].replace(/\\"/g, '"'));
			}
			if (strings.length >= 2) {
				// Parse validity annotation: ;; @valid[2024-01-01T00:00:00Z, inf]
				let validFrom: string | null = null;
				let validTo: string | null = null;
				const validMatch = trimmed.match(/;;\s*@valid\[([^,]+),\s*([^\]]+)\]/);
				if (validMatch) {
					validFrom = validMatch[1].trim();
					const to = validMatch[2].trim();
					validTo = to === 'inf' ? null : to;
				}
				facts.push({
					predicate: strings[0],
					terms: strings.slice(1),
					kind: 'base',
					confidence: null,
					source: null,
					validFrom,
					validTo
				});
			}
		}
	}
	return facts;
}

// ---------------------------------------------------------------------------
// Branches
// ---------------------------------------------------------------------------

export interface BranchRow {
	branch_id: string;
	name: string;
	parent_branch_id: string | null;
	created_tx_id: number;
	archived: boolean;
	is_current: boolean;
	fact_count: number;
}

export interface BranchDiffResult {
	added: Record<string, unknown>[];
	removed: Record<string, unknown>[];
	changed: Record<string, unknown>[];
}

export interface MergeBranchResult {
	added: string[];
	conflicts: Array<{
		fact_id: string;
		predicate: string;
		source_value: string;
		target_value: string;
	}>;
	tx_id: number;
}

async function deleteRequest(path: string): Promise<void> {
	const { signal, clear } = signalWithTimeout(DEFAULT_FETCH_TIMEOUT_MS);
	let res: Response;
	try {
		res = await fetch(endpoint(path), { method: 'DELETE', signal });
		clear();
	} catch (e) {
		clear();
		if (signal.aborted) throw new Error(fetchTimedOutMessage());
		throw e instanceof Error ? e : new Error(String(e));
	}
	if (!res.ok) throw new Error(`Delete failed: ${res.status} ${res.statusText}`);
}

export async function fetchBranches(exom = DEFAULT_EXOM): Promise<BranchRow[]> {
	const r = await readJson<{ branches: BranchRow[] }>(
		`api/branches?exom=${encodeURIComponent(exom)}`
	);
	return r.branches;
}

export async function createBranch(
	branchId: string,
	name: string,
	exom = DEFAULT_EXOM
): Promise<{ branch_id: string; tx_id: number }> {
	return postAction(`api/branches?exom=${encodeURIComponent(exom)}`, {
		branch_id: branchId,
		name
	});
}

export async function switchBranch(branchId: string, exom = DEFAULT_EXOM): Promise<void> {
	await postText(`api/branches/${encodeURIComponent(branchId)}/switch?exom=${encodeURIComponent(exom)}`, '');
}

export async function fetchBranchDiff(
	branchId: string,
	base: string,
	exom = DEFAULT_EXOM
): Promise<BranchDiffResult> {
	return readJson(
		`api/branches/${encodeURIComponent(branchId)}/diff?exom=${encodeURIComponent(exom)}&base=${encodeURIComponent(base)}`
	);
}

export async function mergeBranch(
	source: string,
	policy: string,
	exom = DEFAULT_EXOM
): Promise<MergeBranchResult> {
	return postAction(
		`api/branches/${encodeURIComponent(source)}/merge?exom=${encodeURIComponent(exom)}`,
		{ policy }
	);
}

export async function deleteBranch(branchId: string, exom = DEFAULT_EXOM): Promise<void> {
	await deleteRequest(`api/branches/${encodeURIComponent(branchId)}?exom=${encodeURIComponent(exom)}`);
}
