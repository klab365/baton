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
};
type Run = { steps: { attempt: number; output: string | null }[] };
type Progress = { id: string; status: string };
type Exec = (args: string[]) => Promise<unknown>;
type UI = Pick<
	ExtensionCommandContext["ui"],
	"select" | "editor" | "confirm" | "notify" | "setStatus"
>;

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
				const run = (await exec(["status", "--run", runId])) as Run;
				const context = [...run.steps]
					.reverse()
					.find((step) => step.output)?.output;
				show(
					`Baton ${runId}: human decision for ${item.step.name}\n\n${context ?? "No prior handoff."}`,
				);
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
				show(`Baton ${runId}: ${item.status}`);
				ui.notify(
					`Baton ${runId}: ${item.status}`,
					item.status === "succeeded" ? "info" : "warning",
				);
				return;
			}
			if (!item.step || item.attempt === null)
				throw new Error("Invalid running work item");
			ui.setStatus(
				"baton",
				`Baton: ${item.step.name} (attempt ${item.attempt})`,
			);
			show(
				`Baton ${runId}: executing ${item.step.name} (attempt ${item.attempt})`,
			);
			await exec(["execute", "--run", runId]);
			const run = (await exec(["status", "--run", runId])) as Run;
			const output = run.steps.find(
				(step) => step.attempt === item.attempt,
			)?.output;
			if (output) show(`Baton ${runId}: ${item.step.name} result\n\n${output}`);
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
				if (!runId) {
					const task = await ctx.ui.editor(
						"Baton task (include issue requirements and acceptance criteria)",
						"",
					);
					if (!task?.trim()) return;
					const pipeline = await ctx.ui.input(
						"Pipeline ID (leave empty if the project has one pipeline)",
						"",
					);
					if (pipeline === undefined) return;
					const startArgs = ["start", "--issue", task];
					if (pipeline.trim()) startArgs.push("--pipeline", pipeline.trim());
					const started = (await exec(startArgs)) as Progress;
					runId = started.id;
					show(`Baton run started: ${runId}`);
				}
				await drive(runId, exec, ctx.ui, show);
			} catch (error) {
				ctx.ui.notify(
					`Baton${runId ? ` ${runId}` : ""}: ${error instanceof Error ? error.message : String(error)}`,
					"error",
				);
			}
		},
	});
}
