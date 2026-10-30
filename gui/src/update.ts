// "A newer docex is available": at most one check a day against GitHub's
// latest release. Nothing is sent but the request itself; the person
// downloads the new version from the release page.

import { invoke } from "@tauri-apps/api/core";
import { getVersion } from "@tauri-apps/api/app";

export interface Update {
  version: string;
  url: string;
}

interface Cache {
  at: number;
  /** The version that checked: a cached result is stale once it changes. */
  current: string;
  found: Update | null;
}

const KEY = "docex-update";
const OFF = "docex-update-off";
const DAY = 24 * 3600 * 1000;

function read<T>(k: string): T | null {
  try {
    return JSON.parse(localStorage.getItem(k) ?? "null") as T | null;
  } catch {
    return null;
  }
}

function write(k: string, v: unknown) {
  try {
    localStorage.setItem(k, JSON.stringify(v));
  } catch {
    /* private mode: check again next time */
  }
}

export const updateChecksOn = () => !read<boolean>(OFF);

export function setUpdateChecks(on: boolean) {
  write(OFF, !on);
  if (!on) write(KEY, null);
}

/** The newer release, if any. `force` asks GitHub even if it was asked today
 *  (and throws if that fails); otherwise errors just mean "no notice". */
export async function checkUpdate(force = false): Promise<Update | null> {
  if (!force && !updateChecksOn()) return null;
  const current = await getVersion().catch(() => "");
  const cache = read<Cache>(KEY);
  if (!force && cache && cache.current === current && Date.now() - cache.at < DAY) return cache.found;
  try {
    const found = await invoke<Update | null>("check_update");
    write(KEY, { at: Date.now(), current, found } satisfies Cache);
    return found;
  } catch (e) {
    if (force) throw e;
    return null;
  }
}

/** Don't show the notice for this version again. */
export function dismissUpdate(u: Update) {
  write(`${KEY}-dismissed`, u.version);
}

export const dismissed = (u: Update) => read<string>(`${KEY}-dismissed`) === u.version;

export const openRelease = (u: Update) => invoke("open_release", { url: u.url });
