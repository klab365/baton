export type ActivityItem = { label: string; content: string };

// Transcripts are runtime-specific; unknown shapes remain available in raw diagnostics.
export function parseRunActivity(content: string): ActivityItem[] {
	try {
		const transcript: unknown = JSON.parse(content);
		const messages = Array.isArray(transcript)
			? transcript
			: (transcript as { messages?: unknown[] })?.messages;
		if (!Array.isArray(messages)) return [];
		const items: ActivityItem[] = [];
		for (const message of messages) {
			if (!message || !Array.isArray(message.content)) continue;
			for (const part of message.content) {
				if (part?.type === "toolcall") {
					items.push({
						label: `Tool call: ${part.function?.name ?? "unknown"}`,
						content: JSON.stringify(part.function?.arguments ?? {}, null, 2),
					});
				} else if (part?.type === "toolresult") {
					items.push({
						label: `Tool result: ${part.name ?? "unknown"}`,
						content: JSON.stringify(part.content ?? [], null, 2),
					});
				} else if (
					message.role === "assistant" &&
					part?.type === "text" &&
					typeof part.text === "string" &&
					part.text.trim()
				) {
					items.push({ label: "Agent response", content: part.text });
				}
			}
		}
		return items;
	} catch {
		return [];
	}
}
