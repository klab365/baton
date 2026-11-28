<script lang="ts">
let { status }: { status?: string | null } = $props();
const value = $derived(status ?? "unknown");

const labels: Record<string, string> = {
	waitingHuman: "Review pending",
	limitExhausted: "Limit reached",
	doing: "Working",
	done: "Done",
	draft: "Draft",
};
function tone(value: string) {
	if (["waitingHuman", "waiting", "approval"].includes(value)) return "waiting";
	if (["failed", "limitExhausted", "cancelled"].includes(value))
		return "failed";
	if (["succeeded", "done", "completed"].includes(value)) return "success";
	if (["running", "doing", "working"].includes(value)) return "running";
	return "neutral";
}
function label(value: string) {
	return (labels[value] ?? value)
		.replaceAll(/([A-Z])/g, " $1")
		.replace(/^./, (letter) => letter.toUpperCase());
}
</script>

<span class="status-badge status-badge-{tone(value)}">{label(value)}</span>
