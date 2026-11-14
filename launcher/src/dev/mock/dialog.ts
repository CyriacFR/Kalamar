// Browser stand-in for "@tauri-apps/plugin-dialog" (vite --mode mock only): a file picker that answers a made-up path.
export async function open(options?: { directory?: boolean; filters?: { extensions: string[] }[] }): Promise<string | null> {
  if (options?.directory) return "D:\\Games\\Kalamar";
  const ext = options?.filters?.[0]?.extensions?.[0] ?? "bin";
  return `D:\\Files\\example.${ext}`;
}
