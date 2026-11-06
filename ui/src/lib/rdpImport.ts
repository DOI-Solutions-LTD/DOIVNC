/**
 * The rules for turning an imported `.rdp` draft into a library entry.
 *
 * Pure, so they are tested on their own (`rdpImport.test.ts`); the library
 * does the saving and connecting.
 */
import type { HostProfile } from "./types";

/**
 * The library name for an imported file.
 *
 * Browsers number repeat downloads `name (1).rdp`, `name (2).rdp`, and RDP
 * gateways that hand out one-time files (Fudo PAM, for one) are downloaded
 * again for every session. Dropping that counter is what lets the twentieth
 * download land on the same library entry as the first.
 */
export function importedName(friendlyName: string): string {
  const name = friendlyName.replace(/\s*\(\d+\)$/, "").trim();
  return name || friendlyName;
}

/** `DOMAIN\user` split the way the host editor stores it; anything else is a bare user. */
export function splitRdpUser(username: string | null | undefined): { user: string; domain: string } {
  const value = (username ?? "").trim();
  const slash = value.indexOf("\\");
  if (slash > 0 && slash < value.length - 1) {
    return { domain: value.slice(0, slash), user: value.slice(slash + 1) };
  }
  return { user: value, domain: "" };
}

/**
 * The saved host an imported file is another copy of: an RDP host with the
 * same name, address and port.
 *
 * Re-importing such a file only refreshes the stored user name, so settings
 * a person changed on that host (clipboard, sound, quality) survive a gateway
 * handing out a new one-time file for every session.
 */
export function findExistingHost(
  hosts: readonly HostProfile[],
  profile: Pick<HostProfile, "address" | "port">,
  name: string,
): HostProfile | undefined {
  const address = profile.address.trim().toLowerCase();
  return hosts.find(
    (h) =>
      (h.protocol || "vnc") === "rdp" &&
      h.address.trim().toLowerCase() === address &&
      h.port === profile.port &&
      h.friendlyName === name,
  );
}
