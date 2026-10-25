export async function load({ fetch }: { fetch: typeof globalThis.fetch }) {
	const response = await fetch("/api/tools");
	if (!response.ok) throw new Error("Could not load built-in tools.");
	return { tools: await response.json() };
}
