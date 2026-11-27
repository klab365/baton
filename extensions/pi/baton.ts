import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import net from "node:net";
import type {
	ExtensionAPI,
	ExtensionCommandContext,
} from "@earendil-works/pi-coding-agent";

type WorkItem = {
	runId: string;
	revision: number;
	status: string;
	step: { id: string; name: string; human: boolean } | null;
	attempt: number | null;
	allowedOutcomes: string[];
	lastResult: { stepId: string; attempt: number; output: string } | null;
};
type Progress = { id: string; status: string };
type Exec = (args: string[]) => Promise<unknown>;
let herdrSeq = Date.now() * 1000;
export function herdrRequest(method: string, params: Record<string, unknown>): void {
	const socketPath = process.env.HERDR_SOCKET_PATH;
	const paneId = process.env.HERDR_PANE_ID;
	if (process.env.HERDR_ENV !== "1" || !socketPath || !paneId) return;
	const endpoint = process.platform === "win32" ? `\\\\.\\pipe\\${socketPath}` : socketPath;
	try {
		const socket = net.createConnection(endpoint);
		socket.setTimeout(500, () => socket.destroy());
		socket.on("error", () => {});
		socket.on("connect", () => socket.end(`${JSON.stringify({ id: `baton:${Date.now()}`, method, params: { pane_id: paneId, ...params } })}\n`));
	} catch {
		// Herdr is optional; malformed local IPC configuration must not break Baton.
	}
}

type UI = Pick<
	ExtensionCommandContext["ui"],
	"select" | "editor" | "confirm" | "notify" | "setStatus"
>;

export async function choosePipeline(exec: Exec, ui: UI): Promise<string | undefined> {
	const pipelines = (await exec(["pipelines"])) as string[];
	if (pipelines.length === 0) {
		ui.notify("Baton: no pipelines are configured", "warning");
		return;
	}
	return ui.select("Baton: choose a pipeline", pipelines);
}

function boundedText(text: string): string {
	return text.length > 8000 ? `${text.slice(0, 7970)}\n… inspection output truncated` : text;
}

function listRecords(value: unknown, page: number): string {
	if (!Array.isArray(value)) throw new Error("Invalid work session list");
	const records = value.map((run) => {
		if (!run || typeof run !== "object") return {};
		const source = run as Record<string, unknown>;
		return Object.fromEntries(["id", "status", "currentStep", "attempt"]
			.filter((key) => source[key] !== undefined && (source[key] === null || ["string", "number", "boolean"].includes(typeof source[key])))
			.map((key) => [key, typeof source[key] === "string" && source[key].length > 160 ? `${source[key].slice(0, 160)}… ${key} truncated` : source[key]]));
	});
	const pages: typeof records[] = [];
	let current: typeof records = [];
	for (const record of records) {
		const candidate = [...current, record];
		if (current.length === 50 || (current.length && JSON.stringify(candidate, null, 2).length > 7000)) {
			pages.push(current);
			current = [record];
		} else current = candidate;
	}
	if (current.length || !pages.length) pages.push(current);
	if (!Number.isSafeInteger(page) || page < 1) throw new Error("List page must be a positive integer");
	if (page > pages.length) throw new Error(`Page ${page} does not exist; list has ${pages.length} page(s)`);
	const navigation = `Page ${page} of ${pages.length}${page > 1 ? ` — previous: /baton list ${page - 1}` : ""}${page < pages.length ? ` — next: /baton list ${page + 1}` : ""}`;
	return boundedText(`Work sessions — ${navigation}\n${JSON.stringify(pages[page - 1], null, 2)}`);
}

function formatInsights(steps: Record<string, unknown>[]) {
	const executed = steps.filter((step) => step.status !== "pending" || Number(step.attempt) > 0);
	const retries = executed.reduce((count, step, index) => count + (executed.slice(0, index).some((prior) => prior.stepId === step.stepId) ? 1 : 0), 0);
	const total = (key: string) => {
		const values = executed.map((step) => (step.usage as Record<string, unknown> | undefined)?.[key]).filter((value): value is number => typeof value === "number");
		return `${values.length ? values.reduce((sum, value) => sum + value, 0) : "unavailable"} (${values.length === 0 ? "unavailable" : values.length === executed.length ? "complete" : "partial"})`;
	};
	const durations = executed.map((step) => step.durationMs).filter((value): value is number => typeof value === "number");
	const prices = executed.flatMap((step) => {
		const usage = step.usage as Record<string, unknown> | undefined;
		const value = usage?.estimatedCost;
		return typeof value === "string" && value.trim() && Number.isFinite(Number(value)) ? [{ value, source: usage?.priceSource, checked: usage?.priceCheckedAt }] : [];
	});
	const costSummary = prices.slice(0, 3).map((price) => `${price.value}${typeof price.source === "string" ? ` · ${price.source.slice(0, 80)}` : ""}${typeof price.checked === "string" ? ` · checked ${price.checked.slice(0, 80)}` : ""}`).join("; ");
	return `Attempts: ${executed.length} · retries: ${retries}\nDuration: ${durations.length ? durations.reduce((sum, value) => sum + value, 0) : "unavailable"} ms (${durations.length === 0 ? "unavailable" : durations.length === executed.length ? "complete" : "partial"})\nTokens input/output/cache read/cache write: ${total("inputTokens")} / ${total("outputTokens")} / ${total("cacheReadTokens")} / ${total("cacheWriteTokens")}\nRun cost: unavailable (currency/unit not recorded)\nAttempt costs: ${costSummary || "unavailable"}${prices.length > 3 ? `; ${prices.length - 3} further observations in attempt details` : ""}`;
}

function inspectRecord(value: unknown): string {
	if (!value || typeof value !== "object") throw new Error("Invalid work record");
	const run = value as Record<string, unknown>;
	const allSteps = Array.isArray(run.steps) ? run.steps : [];
	const recorded = allSteps
		.map((value, index) => ({ value, index }))
		.filter(({ value }) => value && typeof value === "object" && (value.status !== "pending" || Number(value.attempt) > 0))
		.sort((a, b) => {
			const aAttempt = Number((a.value as Record<string, unknown>).attempt);
			const bAttempt = Number((b.value as Record<string, unknown>).attempt);
			return aAttempt > 0 && bAttempt > 0 ? aAttempt - bAttempt || a.index - b.index : aAttempt > 0 ? 1 : bAttempt > 0 ? -1 : a.index - b.index;
		});
	const visible = recorded.slice(-30).reverse();
	const steps = visible.map(({ value }) => {
		if (!value || typeof value !== "object") return {};
		const step = value as Record<string, unknown>;
		const record = Object.fromEntries(["stepId", "status", "attempt", "outcome", "startedAt", "endedAt", "durationMs", "output", "error", "model"]
			.filter((key) => step[key] !== undefined)
			.map((key) => [key, typeof step[key] === "string" && step[key].length > 1000 ? `${step[key].slice(0, 1000)}… output truncated` : step[key]]));
		if (step.usage && typeof step.usage === "object") {
			record.usage = Object.fromEntries(["inputTokens", "outputTokens", "cacheReadTokens", "cacheWriteTokens", "estimatedCost", "priceSource", "priceCheckedAt"]
				.filter((key) => (step.usage as Record<string, unknown>)[key] !== undefined)
				.map((key) => {
					const value = (step.usage as Record<string, unknown>)[key];
					return [key, typeof value === "string" && value.length > 160 ? `${value.slice(0, 160)}… truncated` : value];
				}));
		}
		return record;
	}) as unknown[];
	const record: Record<string, unknown> = Object.fromEntries(["id", "status", "pipelineId", "currentStep", "startedAt", "endedAt", "durationMs"]
		.filter((key) => run[key] !== undefined).map((key) => [key, run[key]]));
	if (recorded.length > steps.length) record.omittedEarlierSteps = `${recorded.length - steps.length} earlier steps omitted`;
	record.steps = steps;
	return boundedText(`Work session ${String(run.id ?? "unknown")} · ${String(run.status ?? "status unavailable")}\n${formatInsights(allSteps as Record<string, unknown>[])}\n${JSON.stringify(record, null, 2)}`);
}

async function chooseRefinementRun(
	exec: Exec,
	id: string | undefined,
	ui: UI,
): Promise<string | undefined> {
	const value = await exec(["status"]);
	if (!Array.isArray(value)) throw new Error("Invalid work session list");
	const candidates = value
		.filter(
			(run): run is Record<string, unknown> =>
				!!run && typeof run === "object" &&
				typeof run.id === "string" && run.status === "succeeded",
		)
		.sort((a, b) => {
			const startedAt = (run: Record<string, unknown>) => {
				if (typeof run.startedAt !== "string") return 0;
				const timestamp = Date.parse(run.startedAt);
				return Number.isFinite(timestamp) ? timestamp : 0;
			};
			return startedAt(b) - startedAt(a);
		});
	if (id) {
		const run = candidates.find((candidate) => candidate.id === id);
		if (!run) throw new Error(`No completed work session with ID '${id}'`);
		return run.id as string;
	}
	if (!candidates.length) throw new Error("No completed work sessions to refine");
	const labels = candidates.map(
		(run, index) => `${index === 0 ? "★ " : ""}${String(run.id)} · ${typeof run.pipelineId === "string" ? run.pipelineId : "pipeline"}`,
	);
	const selected = await ui.select("Baton: choose a completed run to refine", labels);
	const run = candidates[labels.indexOf(selected ?? "")];
	return run?.id as string | undefined;
}

export async function manage(
	args: string,
	exec: Exec,
	ui: UI,
	show: (text: string, notificationKey?: string) => void,
): Promise<boolean> {
	const [command, id, ...extra] = args.trim().split(/\s+/);
	if (!command) return false;
	if (command === "help" && !id) {
		show("/baton — start a task; /baton RUN_ID — resume. Commands: list [PAGE], inspect RUN_ID, resume RUN_ID, refine [RUN_ID], help. Refine starts a new run in this worktree and reuses the prior run's agent conversations. List and inspect are read-only; resume and refine may execute configured steps and ask for human decisions. Live Pi subagent activity is not captured.");
		return true;
	}
	if (command === "list" && (!id || (!extra.length && /^\d+$/.test(id)))) {
		const page = id ? Number(id) : 1;
		if (!Number.isSafeInteger(page) || page < 1) throw new Error("List page must be a positive integer");
		show(listRecords(await exec(["status"]), page));
		return true;
	}
	if (command === "refine" && extra.length === 0) {
		const runId = await chooseRefinementRun(exec, id, ui);
		if (!runId) return true;
		const instruction = await ui.editor("Baton refinement: what should change or be done next?", "");
		if (!instruction?.trim()) return true;
		const started = (await exec(["refine", "--run", runId, "--instruction", instruction])) as {
			id?: string;
		};
		if (typeof started.id !== "string" || !started.id) throw new Error("Invalid refined work session");
		show(`Baton refinement started: ${started.id} (from ${runId})`, `start:${started.id}`);
		await drive(started.id, exec, ui, show);
		return true;
	}
	if ((command === "inspect" || command === "resume") && id && !extra.length) {
		if (command === "inspect") show(inspectRecord(await exec(["status", "--run", id])));
		else await drive(id, exec, ui, show);
		return true;
	}
	throw new Error("Usage: /baton [help|list [PAGE]|inspect RUN_ID|resume RUN_ID|refine [RUN_ID]]");
}

/** Run the CLI state machine; the UI never chooses a human outcome on its own. */
export async function drive(
	runId: string,
	exec: Exec,
	ui: UI,
	show: (text: string, notificationKey?: string) => void,
): Promise<void> {
	try {
		for (;;) {
			const item = (await exec(["next", "--run", runId])) as WorkItem;
			if (item.status === "waitingHuman") {
				if (
					!item.step ||
					item.attempt === null ||
					item.allowedOutcomes.length === 0
				)
					throw new Error("Invalid human work item");
				const context = item.lastResult?.output;
				show(
					`Baton ${runId} — Waiting for your decision\nStep: ${item.step.name}\nBlocked on: ${item.allowedOutcomes.join(", ")}\n\nChoose a decision above, enter a response, and confirm to continue. To leave it waiting, cancel; resume later with /baton resume ${runId}.\n\nPrior handoff:`,
					`human:${runId}:${item.step.id}:${item.attempt}`,
				);
				const handoff = context ?? "No prior handoff.";
				for (let offset = 0; offset < handoff.length; offset += 7000) {
					show(handoff.slice(offset, offset + 7000));
					if (offset + 7000 < handoff.length) {
						const next = await ui.select("Baton: continue reading handoff?", ["Next handoff section", "Continue to decision"]);
					if (next === undefined) return;
					if (next === "Continue to decision") break;
				}
				}
				ui.setStatus("baton", `Baton: waiting for ${item.step.name}`);
				const outcome = await ui.select(
					"Baton: choose a decision (Esc leaves the run waiting)",
					item.allowedOutcomes,
				);
				if (outcome === undefined) return;
				if (!item.allowedOutcomes.includes(outcome))
					throw new Error("Invalid human decision");
				const response = await ui.editor(
					"Baton: response (Esc leaves the run waiting)",
					"",
				);
				if (response === undefined) return;
				if (
					!(await ui.confirm(
						"Confirm Baton decision",
						`Submit ${outcome} for ${item.step.name}?\n\n${response}`,
					))
				)
					return;
				const dir = await mkdtemp(join(tmpdir(), "baton-response-"));
				try {
					const path = join(dir, "response.json");
					await writeFile(
						path,
						JSON.stringify({
							revision: item.revision,
							stepId: item.step.id,
							attempt: item.attempt,
							outcome,
							response,
						}),
						{ mode: 0o600 },
					);
					await exec(["respond", "--run", runId, "--result", path]);
				} finally {
					await rm(dir, { recursive: true, force: true });
				}
				continue;
			}
			if (item.status !== "running") {
				const message = `Baton ${runId} — ${item.status.toUpperCase()}`;
				show(message, `terminal:${runId}:${item.status}`);
				ui.notify(message, item.status === "succeeded" ? "info" : "warning");
				return;
			}
			if (!item.step || item.attempt === null)
				throw new Error("Invalid running work item");
			ui.setStatus(
				"baton",
				`Baton · RUNNING · ${item.step.name} · attempt ${item.attempt}`,
			);
			show(`Baton ${runId} — RUNNING\nStep: ${item.step.name}\nAttempt: ${item.attempt}`);
			let polling = false;
			let active = true;
			let frame = 0;
			let progressStatus = `Baton · RUNNING · ${item.step.name} · attempt ${item.attempt}`;
			const frames = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
			const animationTimer = setInterval(() => {
				if (active) ui.setStatus("baton", `${frames[frame++ % frames.length]} ${progressStatus}`);
			}, 100);
			const timer = setInterval(() => {
				if (!active || polling) return;
				polling = true;
				void exec(["progress", "--run", runId])
					.then((value) => {
						if (!active) return;
						const progress = value as { activity?: unknown; step?: unknown; attempt?: unknown };
						const step = typeof progress.step === "string" ? progress.step.slice(0, 80) : item.step!.name;
						const attempt = typeof progress.attempt === "number" ? progress.attempt : item.attempt;
						const loopActivity = typeof progress.activity === "string" && /^Ralph loop \d+\/\d+$/.test(progress.activity);
						const activity = loopActivity ? progress.activity : ["Tool activity", "Agent activity", "Runtime activity"].includes(progress.activity)
							? progress.activity : "Runtime activity";
						const detail = typeof progress.activity === "string" ? activity : "Activity unavailable";
						const truncated = progress.truncated === true ? " · earlier activity omitted" : "";
						progressStatus = `Baton · RUNNING · ${step} · attempt ${attempt} · ${detail}${truncated}`;
						ui.setStatus("baton", `${frames[frame++ % frames.length]} ${progressStatus}`);
					})
					.catch(() => {})
					.finally(() => { polling = false; });
			}, 1000);
			try {
				await exec(["execute", "--run", runId]);
			} finally {
				active = false;
				clearInterval(timer);
				clearInterval(animationTimer);
			}
			const next = (await exec(["next", "--run", runId])) as WorkItem;
			const output =
				next.lastResult?.attempt === item.attempt
					? next.lastResult.output
					: undefined;
			if (output) show(`Baton ${runId} — RESULT · ${item.step.name}\n\n${output}`, `result:${runId}:${item.step.id}:${item.attempt}`);
		}
	} finally {
		ui.setStatus("baton", undefined);
	}
}

export default function baton(pi: ExtensionAPI) {
	let activityTimer: ReturnType<typeof setInterval> | undefined;
	let activityFrame = 0;
	pi.on("agent_start", async (event, ctx) => {
		if (activityTimer) clearInterval(activityTimer);
		activityFrame = 0;
		const frames = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
		ctx.ui.setStatus("baton", "⠋ Pi · working");
		herdrRequest("pane.report_agent", { source: "baton:pi-turn", agent: "pi", state: "working", seq: ++herdrSeq });
		activityTimer = setInterval(() => {
			ctx.ui.setStatus("baton", `${frames[++activityFrame % frames.length]} Pi · working`);
		}, 100);
	});
	pi.on("agent_end", async (_event, ctx) => {
		if (activityTimer) clearInterval(activityTimer);
		activityTimer = undefined;
		ctx.ui.setStatus("baton", undefined);
		herdrRequest("pane.report_agent", { source: "baton:pi-turn", agent: "pi", state: "idle", seq: ++herdrSeq });
	});
	pi.on("message_end", async (event) => {
		const message = event.message as { role?: string; stopReason?: string } | undefined;
		if (message?.role === "assistant" && message.stopReason === "stop") {
			herdrRequest("notification.show", { title: "Pi", body: "New assistant response" });
		}
	});
	const notified = new Set<string>();
	pi.registerCommand("baton", {
		description: "Start, resume or refine a Baton run in this worktree",
		handler: async (args, ctx) => {
			if (!ctx.hasUI) return;
			const exec: Exec = async (workArgs) => {
				const result = await pi.exec("baton", ["run", ...workArgs], {
					cwd: ctx.cwd,
					maxBuffer: 16 * 1024 * 1024,
				});
				if (result.code !== 0 || result.killed || result.truncated)
					throw new Error(
						result.stderr || "Baton command failed or output was truncated",
					);
				try {
					return JSON.parse(result.stdout);
				} catch {
					throw new Error("Baton returned invalid JSON");
				}
			};
			const show = (text: string, notificationKey?: string) => {
				pi.sendMessage({ customType: "baton", content: text, display: true });
				if (notificationKey && !notified.has(notificationKey)) {
					notified.add(notificationKey);
					herdrRequest("notification.show", { title: "Baton", body: "New Baton conversation update" });
				}
			};
			let runId = args.trim();
			herdrRequest("pane.report_agent", { source: "baton:pi", agent: "pi", state: "working", seq: ++herdrSeq });
			try {
				if (/^(help|list|inspect|resume|refine)(?:\s|$)/.test(runId)) {
					await manage(runId, exec, ctx.ui, show);
					return;
				}
				if (!runId) {
					const task = await ctx.ui.editor(
						"Baton task (include issue requirements and acceptance criteria)",
						"",
					);
					if (!task?.trim()) return;
					const pipeline = await choosePipeline(exec, ctx.ui);
					if (pipeline === undefined) return;
					const startArgs = ["start", "--issue", task, "--pipeline", pipeline];
					const started = (await exec(startArgs)) as Progress;
					runId = started.id;
					show(`Baton run started: ${runId}`, `start:${runId}`);
				}
				await drive(runId, exec, ctx.ui, show);
			} catch (error) {
				ctx.ui.notify(
					`Baton${runId ? ` ${runId}` : ""} — ERROR\n${error instanceof Error ? error.message : String(error)}`,
					"error",
				);
			} finally {
				herdrRequest("pane.report_agent", { source: "baton:pi", agent: "pi", state: "idle", seq: ++herdrSeq });
			}
		},
	});
}
