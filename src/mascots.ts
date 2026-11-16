import { invoke } from "@tauri-apps/api/core";
import type { Language } from "./prefs";
import lanbaoIdle from "./assets/mascot/lanbao/idle.png";
import lanbaoShrug from "./assets/mascot/lanbao/shrug.png";
import lanbaoDizzy from "./assets/mascot/lanbao/dizzy.png";
import lanbaoPoint from "./assets/mascot/lanbao/point.png";
import lanbaoHappy from "./assets/mascot/lanbao/happy.png";
import lanbaoTap from "./assets/mascot/lanbao/tap.png";

export type MascotPose = "idle" | "shrug" | "dizzy" | "point" | "happy" | "tap";

export const POSE_NAMES: MascotPose[] = ["idle", "shrug", "dizzy", "point", "happy", "tap"];

export type Mascot = {
  id: string;
  name: Record<Language, string>;
  thumb: string;
  poses: Record<MascotPose, string>;
};

export const mascots: Mascot[] = [
  {
    id: "lanbao",
    name: { zh: "蓝宝", en: "Lanbao" },
    thumb: lanbaoIdle,
    poses: {
      idle: lanbaoIdle,
      shrug: lanbaoShrug,
      dizzy: lanbaoDizzy,
      point: lanbaoPoint,
      happy: lanbaoHappy,
      tap: lanbaoTap,
    },
  },
];

const remote = new Map<string, Mascot>();

export function registerRemoteMascot(mascot: Mascot) {
  remote.set(mascot.id, mascot);
}

export function hasMascot(id: string): boolean {
  return mascots.some((mascot) => mascot.id === id) || remote.has(id);
}

export function isBundled(id: string): boolean {
  return mascots.some((mascot) => mascot.id === id);
}

export function mascotById(id: string): Mascot {
  return mascots.find((mascot) => mascot.id === id) ?? remote.get(id) ?? mascots[0];
}

export function dataUrl(base64: string): string {
  return `data:image/png;base64,${base64}`;
}

export function posesFromCache(cached: Record<string, string>): Record<MascotPose, string> {
  const poses = {} as Record<MascotPose, string>;
  for (const pose of POSE_NAMES) poses[pose] = dataUrl(cached[pose]);
  return poses;
}

export type RemotePose = { file: string; sha256: string; bytes: number };
export type RemoteCharacter = {
  id: string;
  name: Record<Language, string>;
  poses: Record<MascotPose, RemotePose>;
};

const ID_PATTERN = /^[a-z0-9-]{1,24}$/;
const FILE_PATTERN = /^mascots\/[a-z0-9-]+\/[a-z]+\.png$/;
const SHA_PATTERN = /^[0-9a-f]{64}$/;

function validCharacter(value: unknown): value is RemoteCharacter {
  if (!value || typeof value !== "object") return false;
  const entry = value as Record<string, unknown>;
  if (typeof entry.id !== "string" || !ID_PATTERN.test(entry.id)) return false;
  const name = entry.name as Record<string, unknown> | undefined;
  if (!name || typeof name.zh !== "string" || typeof name.en !== "string") return false;
  const poses = entry.poses as Record<string, RemotePose> | undefined;
  if (!poses) return false;
  return POSE_NAMES.every((pose) => {
    const spec = poses[pose];
    return (
      spec &&
      typeof spec.file === "string" &&
      FILE_PATTERN.test(spec.file) &&
      typeof spec.sha256 === "string" &&
      SHA_PATTERN.test(spec.sha256) &&
      typeof spec.bytes === "number" &&
      spec.bytes > 0
    );
  });
}

export async function fetchManifest(): Promise<RemoteCharacter[]> {
  let text: string;
  try {
    text = await invoke<string>("fetch_mascot_manifest");
  } catch (cause) {
    if (String(cause) === "empty") return [];
    throw cause;
  }
  const parsed = JSON.parse(text) as { version?: number; characters?: unknown[] };
  if (parsed.version !== 1 || !Array.isArray(parsed.characters)) {
    throw new Error("bad manifest");
  }
  return parsed.characters.filter(validCharacter);
}

async function sha256Hex(bytes: Uint8Array<ArrayBuffer>): Promise<string> {
  const digest = await crypto.subtle.digest("SHA-256", bytes);
  return [...new Uint8Array(digest)].map((b) => b.toString(16).padStart(2, "0")).join("");
}

export async function ensurePose(
  entry: RemoteCharacter,
  pose: MascotPose,
  cached?: string,
): Promise<string> {
  if (cached) return dataUrl(cached);
  const spec = entry.poses[pose];
  const base64 = await invoke<string>("fetch_mascot_file", { path: spec.file });
  const bytes = Uint8Array.from(atob(base64), (char) => char.charCodeAt(0));
  if (bytes.length !== spec.bytes || (await sha256Hex(bytes)) !== spec.sha256) {
    throw new Error("verify failed");
  }
  await invoke("save_mascot_pose", { id: entry.id, pose, data: base64 });
  return dataUrl(base64);
}

export async function downloadCharacter(entry: RemoteCharacter): Promise<Mascot> {
  const cached = await invoke<Record<string, string>>("read_cached_mascot", { id: entry.id });
  const poses = {} as Record<MascotPose, string>;
  for (const pose of POSE_NAMES) {
    poses[pose] = await ensurePose(entry, pose, cached[pose]);
  }
  const mascot: Mascot = { id: entry.id, name: entry.name, thumb: poses.idle, poses };
  registerRemoteMascot(mascot);
  return mascot;
}