import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
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
	return text.length > 8000 ? `${text.slice(0, 8000)}\n… inspection output truncated` : text;
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

function inspectRecord(value: unknown): string {
	if (!value || typeof value !== "object") throw new Error("Invalid work record");
	const run = value as Record<string, unknown>;
	const allSteps = Array.isArray(run.steps) ? run.steps : [];
	const steps = allSteps.slice(-30).map((value) => {
		if (!value || typeof value !== "object") return {};
		const step = value as Record<string, unknown>;
		return Object.fromEntries(["stepId", "status", "attempt", "outcome", "startedAt", "endedAt", "durationMs", "output", "error"]
			.filter((key) => step[key] !== undefined)
			.map((key) => [key, typeof step[key] === "string" && step[key].length > 1000 ? `${step[key].slice(0, 1000)}… output truncated` : step[key]]));
	}) as unknown[];
	const record: Record<string, unknown> = Object.fromEntries(["id", "status", "pipelineId", "currentStep", "startedAt", "endedAt", "durationMs"]
		.filter((key) => run[key] !== undefined).map((key) => [key, run[key]]));
	if (allSteps.length > steps.length) record.omittedEarlierSteps = `${allSteps.length - steps.length} earlier steps omitted`;
	record.steps = steps;
	return boundedText(`Work session details\n${JSON.stringify(record, null, 2)}`);
}

export async function manage(
	args: string,
	exec: Exec,
	ui: UI,
	show: (text: string) => void,
): Promise<boolean> {
	const [command, id, ...extra] = args.trim().split(/\s+/);
	if (!command) return false;
	if (command === "help" && !id) {
		show("/baton — start a task; /baton RUN_ID — resume. Commands: list [PAGE], inspect RUN_ID, resume RUN_ID, help. List pages contain up to 50 sessions, adapt to an output-size budget, and show previous/next commands. List and inspect read local work-session records; resume may execute configured steps and asks before human decisions. Live Pi subagent activity is not captured.");
		return true;
	}
	if (command === "list" && (!id || (!extra.length && /^\d+$/.test(id)))) {
		const page = id ? Number(id) : 1;
		if (!Number.isSafeInteger(page) || page < 1) throw new Error("List page must be a positive integer");
		show(listRecords(await exec(["status"]), page));
		return true;
	}
	if ((command === "inspect" || command === "resume") && id && !extra.length) {
		if (command === "inspect") show(inspectRecord(await exec(["status", "--run", id])));
		else await drive(id, exec, ui, show);
		return true;
	}
	throw new Error("Usage: /baton [help|list [PAGE]|inspect RUN_ID|resume RUN_ID]");
}

/** Run the CLI state machine; the UI never chooses a human outcome on its own. */
export async function drive(
	runId: string,
	exec: Exec,
	ui: UI,
	show: (text: string) => void,
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
				show(message);
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
			await exec(["execute", "--run", runId]);
			const next = (await exec(["next", "--run", runId])) as WorkItem;
			const output =
				next.lastResult?.attempt === item.attempt
					? next.lastResult.output
					: undefined;
			if (output) show(`Baton ${runId} — RESULT · ${item.step.name}\n\n${output}`);
		}
	} finally {
		ui.setStatus("baton", undefined);
	}
}

export default function baton(pi: ExtensionAPI) {
	pi.registerCommand("baton", {
		description: "Start or resume a Baton run in this workspace",
		handler: async (args, ctx) => {
			if (!ctx.hasUI) return;
			const exec: Exec = async (workArgs) => {
				const result = await pi.exec("baton", ["work", ...workArgs], {
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
			const show = (text: string) =>
				pi.sendMessage({ customType: "baton", content: text, display: true });
			let runId = args.trim();
			try {
				if (/^(help|list|inspect|resume)(?:\s|$)/.test(runId)) {
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
					show(`Baton run started: ${runId}`);
				}
				await drive(runId, exec, ctx.ui, show);
			} catch (error) {
				ctx.ui.notify(
					`Baton${runId ? ` ${runId}` : ""} — ERROR\n${error instanceof Error ? error.message : String(error)}`,
					"error",
				);
			}
		},
	});
}
