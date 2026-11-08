import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { runPi } from "./process";

const limit = 16_384;

export function createExtension(run = runPi) {
	return (pi: ExtensionAPI) => {
		pi.registerCommand("baton-status", {
			description: "Show Baton work runs or details for a run ID",
			async handler(args, ctx) {
				const id = args.trim();
				if (/\s/.test(id)) {
					ctx.ui.notify("Usage: /baton-status [run-id]", "error");
					return;
				}
				try {
					const result = await pi.exec(
						"baton",
						["work", "status", ...(id ? ["--run", id] : [])],
						{ cwd: ctx.cwd, maxBuffer: 4 * 1024 * 1024 },
					);
					if (result.code !== 0 || result.killed || result.truncated)
						throw new Error(
							result.stderr || "command interrupted or output too large",
						);
					const data = JSON.parse(result.stdout);
					const runs = Array.isArray(data) ? data : [data];
					const lines = runs.map(
						(run) =>
							`${run.id}: ${run.status}${run.currentStep ? ` · ${run.currentStep}` : ""}`,
					);
					if (id && runs[0]?.steps) {
						for (const step of runs[0].steps)
							lines.push(
								`  ${step.stepId} #${step.attempt}: ${step.status}${step.outcome ? ` · ${step.outcome}` : ""}${step.error ? ` · ${step.error}` : ""}`,
							);
					}
					ctx.ui.setWidget(
						"baton-status",
						lines.length ? lines : ["No Baton work runs in this workspace."],
					);
				} catch (error) {
					ctx.ui.notify(
						`Baton status failed: ${error instanceof Error ? error.message : String(error)}`,
						"error",
					);
				}
			},
		});
		pi.registerTool({
			name: "baton_subagent",
			label: "Baton subagent",
			description:
				"Execute the current Baton agent step in a separate Pi process. Returns a validated submission; it does not submit or make human decisions.",
			parameters: {
				type: "object",
				properties: {
					runId: { type: "string", description: "Baton work run ID" },
					context: {
						type: "string",
						description:
							"Acceptance criteria and relevant decisions not contained in the original task",
					},
				},
				required: ["runId", "context"],
				additionalProperties: false,
			},
			executionMode: "sequential",
			async execute(_id, params, signal, onUpdate, ctx) {
				async function baton(args: string[]) {
					const result = await pi.exec("baton", ["work", ...args], {
						cwd: ctx.cwd,
						signal,
						maxBuffer: 4 * 1024 * 1024,
					});
					if (result.code !== 0 || result.killed || result.truncated)
						throw new Error(
							`Baton command failed: ${result.stderr || "interrupted or output too large"}`,
						);
					return JSON.parse(result.stdout);
				}
				const next = await baton(["next", "--run", params.runId]);
				if (next.status !== "running" || next.step?.human || !next.agent) {
					throw new Error("Current step is not an active agent step");
				}
				const status = await baton(["status", "--run", params.runId]);
				if (
					status.session?.revision !== next.revision ||
					status.currentStep !== next.step.id ||
					status.status !== next.status
				) {
					throw new Error("Work state changed; fetch next again");
				}
				const history = status.steps
					.filter((step: { output?: string | null }) => step.output != null)
					.map(
						(step: {
							stepId: string;
							attempt: number;
							outcome: string;
							output: string;
						}) => ({
							stepId: step.stepId,
							attempt: step.attempt,
							outcome: step.outcome,
							output: step.output,
						}),
					);
				const prompt = `You are a Baton subagent in a separate session. Execute only the assigned step. Do not invoke baton work, delegate further, make human decisions, commit, or push. The workspace is shared and is not a sandbox. Follow project instructions.\n\nOriginal task:\n${status.task}\n\nRelevant context from the parent:\n${params.context}\n\nPrior completed steps and human decisions:\n${JSON.stringify(history)}\n\nCurrent work item (agent instructions, assigned skills and allowed outcomes):\n${JSON.stringify(next)}\n\nAssigned skill supporting files are under skillsDirectory/<skill-id>/.\n\nReturn ONLY a JSON object with exactly these fields: outcome (one of allowedOutcomes), summary (nonempty string), evidence (1 to 32 nonempty strings). Do not wrap it in markdown.`;
				if (Buffer.byteLength(prompt) > 256 * 1024)
					throw new Error(
						"Subagent context exceeds 256 KiB; cannot safely omit prior decisions",
					);
				const dir = await mkdtemp(join(tmpdir(), "baton-subagent-"));
				try {
					const file = join(dir, "task.txt");
					await writeFile(file, prompt, { mode: 0o600 });
					const args = [
						"--mode",
						"json",
						"--no-session",
						"--no-skills",
						"--exclude-tools",
						"baton_subagent",
					];
					if (ctx.model)
						args.push("--model", `${ctx.model.provider}/${ctx.model.id}`);
					if (ctx.thinkingLevel) args.push("--thinking", ctx.thinkingLevel);
					args.push(`@${file}`);
					const progress = [
						`Baton ${params.runId} · ${next.step.id} · attempt ${next.attempt}`,
					];
					const update = (message?: string) => {
						if (message) progress.push(message);
						if (ctx.hasUI)
							ctx.ui.setStatus(
								"baton",
								`${progress[0]} · ${message ?? "starting"}`,
							);
						if (progress.length > 13) progress.splice(1, 1);
						onUpdate?.({
							content: [{ type: "text", text: progress.join("\n") }],
							details: {},
						});
					};
					update();
					let result: string;
					try {
						result = await run("pi", args, ctx.cwd, signal, update);
					} catch (error) {
						update(
							`Failed: ${error instanceof Error ? error.message : String(error)}`,
						);
						throw error;
					}
					let answer: unknown;
					try {
						answer = JSON.parse(result);
					} catch {
						throw new Error(
							"Subagent returned invalid JSON; do not submit this attempt",
						);
					}
					if (!answer || typeof answer !== "object" || Array.isArray(answer))
						throw new Error("Invalid subagent result");
					const value = answer as Record<string, unknown>;
					const bounded = (x: unknown) =>
						typeof x === "string" &&
						x.trim().length > 0 &&
						Buffer.byteLength(x) <= limit;
					if (
						Object.keys(value).sort().join(",") !==
							"evidence,outcome,summary" ||
						!next.allowedOutcomes.includes(value.outcome) ||
						!bounded(value.summary) ||
						!Array.isArray(value.evidence) ||
						value.evidence.length < 1 ||
						value.evidence.length > 32 ||
						!value.evidence.every(bounded)
					) {
						throw new Error(
							"Subagent returned an invalid outcome, summary, or evidence; do not submit this attempt",
						);
					}
					const submission = {
						revision: next.revision,
						stepId: next.step.id,
						attempt: next.attempt,
						...value,
					};
					update(
						`Result: ${value.outcome} · ${value.summary}\nEvidence: ${(value.evidence as string[]).join("; ")}`,
					);
					return {
						content: [{ type: "text", text: JSON.stringify(submission) }],
						details: {},
					};
				} finally {
					if (ctx.hasUI) ctx.ui.setStatus("baton", undefined);
					await rm(dir, { recursive: true, force: true });
				}
			},
		});
	};
}

export default createExtension();
