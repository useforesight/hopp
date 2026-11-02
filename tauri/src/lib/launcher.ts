import { typedInvoke } from "@/core_payloads";
import type { components } from "@/openapi";

type Teammate = components["schemas"]["BaseUser"];

const LAUNCHER_KEY_STORAGE = "hopp-launcher-key";

const generateLauncherKey = () =>
  Array.from(crypto.getRandomValues(new Uint8Array(32)), (byte) => byte.toString(16).padStart(2, "0")).join("");

const loadLauncherKey = () => {
  try {
    const savedKey = localStorage.getItem(LAUNCHER_KEY_STORAGE);
    if (savedKey) return savedKey;
    const newKey = generateLauncherKey();
    localStorage.setItem(LAUNCHER_KEY_STORAGE, newKey);
    return newKey;
  } catch {
    return generateLauncherKey();
  }
};

/**
 * Proves a call link came from a launcher on this computer, such as Raycast. Any web page can open a
 * hopp:// link, but only this user can read the file the key is published in, so a page can't start
 * a call with your mic on.
 */
const launcherKey = loadLauncherKey();

export const isLauncherKey = (key: string | null) => key === launcherKey;

let lastPublishedDirectory: string | null = null;

/** Tells launchers who can be called. It's written only when something changed, as this runs on every poll. */
export const publishLauncherDirectory = (teammates: Teammate[]) => {
  const contents = JSON.stringify({
    key: launcherKey,
    teammates: teammates.map(({ id, first_name, last_name, avatar_url, is_active }) => ({
      id,
      first_name,
      last_name,
      avatar_url,
      is_active,
    })),
  });
  if (contents === lastPublishedDirectory) return;
  lastPublishedDirectory = contents;
  typedInvoke("write_launcher_directory", { contents }).catch((err) => {
    lastPublishedDirectory = null;
    console.error("Failed to publish the launcher directory:", err);
  });
};
