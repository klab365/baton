import { spawn } from "node:child_process";
import { StringDecoder } from "node:string_decoder";

const MAX_LINE = 4 * 1024 * 1024;
const MAX_DIAGNOSTIC = 16_384;

type Progress = (message: string) => void;

/** Consume Pi's JSONL stream without retaining its transcript. */
export async function runPi(
	command: string,
	args: string[],
	cwd: string,
	signal: AbortSignal | undefined,
	progress: Progress,
): Promise<string> {
	if (signal?.aborted) throw new Error("Subagent aborted");
	return new Promise((resolve, reject) => {
		const child = spawn(command, args, {
			cwd,
			stdio: ["ignore", "pipe", "pipe"],
			detached: process.platform !== "win32",
		});
		const decoder = new StringDecoder("utf8");
		let buffer = "";
		let stderr = "";
		let finalText = "";
		let stopReason = "";
		let errorMessage = "";
		let settled = false;
		let aborted = false;
		let failure = "";
		let killTimer: ReturnType<typeof setTimeout> | undefined;
		const kill = (sig: NodeJS.Signals) => {
			try {
				if (process.platform !== "win32" && child.pid)
					process.kill(-child.pid, sig);
				else child.kill(sig);
			} catch {
				/* Process already exited. */
			}
		};
		const stop = () => {
			kill("SIGTERM");
			killTimer ??= setTimeout(() => kill("SIGKILL"), 5000);
		};
		const abort = () => {
			aborted = true;
			stop();
		};
		signal?.addEventListener("abort", abort, { once: true });
		if (signal?.aborted) abort();
		const line = (raw: string) => {
			if (!raw.trim() || failure) return;
			let event: {
				type?: string;
				toolName?: string;
				isError?: boolean;
				aborted?: boolean;
				message?: {
					role?: string;
					stopReason?: string;
					errorMessage?: string;
					content?: { type: string; text?: string }[];
				};
			};
			try {
				event = JSON.parse(raw);
				if (!event || typeof event !== "object" || Array.isArray(event))
					throw new Error("Invalid event");
			} catch {
				failure = "Invalid JSON from subagent";
				stop();
				return;
			}
			if (
				event.type === "tool_execution_start" &&
				typeof event.toolName === "string"
			)
				progress(`→ ${event.toolName.slice(0, 120)}`);
			if (
				event.type === "tool_execution_end" &&
				typeof event.toolName === "string"
			)
				progress(
					`${event.isError ? "✗" : "✓"} ${event.toolName.slice(0, 120)}`,
				);
			if (event.type === "auto_retry_start")
				progress("Subagent retrying model request…");
			if (event.type === "message_end" && event.message?.role === "assistant") {
				stopReason = event.message.stopReason ?? "";
				errorMessage =
					typeof event.message.errorMessage === "string"
						? event.message.errorMessage.slice(0, MAX_DIAGNOSTIC)
						: "";
				finalText = (
					Array.isArray(event.message.content) ? event.message.content : []
				)
					.filter((part) => part.type === "text")
					.map((part) => part.text ?? "")
					.join("\n");
			}
			if (event.type === "agent_settled") {
				settled = !event.aborted;
				if (event.aborted) aborted = true;
			}
		};
		child.stdout.on("data", (chunk: Buffer) => {
			buffer += decoder.write(chunk);
			let index = buffer.indexOf("\n");
			while (index !== -1) {
				const raw = buffer.slice(0, index);
				buffer = buffer.slice(index + 1);
				if (Buffer.byteLength(raw) > MAX_LINE) {
					failure = "Subagent event exceeds output limit";
					stop();
					return;
				}
				line(raw);
				index = buffer.indexOf("\n");
			}
			if (Buffer.byteLength(buffer) > MAX_LINE) {
				failure = "Subagent event exceeds output limit";
				buffer = "";
				stop();
			}
		});
		child.stderr.on("data", (chunk: Buffer) => {
			if (stderr.length < MAX_DIAGNOSTIC)
				stderr += chunk.toString().slice(0, MAX_DIAGNOSTIC - stderr.length);
		});
		child.on("error", (error) => {
			failure = error.message;
		});
		child.on("close", (code) => {
			signal?.removeEventListener("abort", abort);
			if (killTimer) clearTimeout(killTimer);
			buffer += decoder.end();
			if (buffer.trim()) line(buffer);
			if (aborted) return reject(new Error("Subagent aborted"));
			if (failure || code !== 0 || !settled || stopReason !== "stop")
				return reject(
					new Error(
						`Subagent failed: ${failure || errorMessage || stderr || `exit ${code}, stop reason ${stopReason || "unknown"}`}`,
					),
				);
			resolve(finalText);
		});
	});
}
