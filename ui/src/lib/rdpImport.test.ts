import { describe, expect, it } from "vitest";
import { findExistingHost, importedName, splitRdpUser } from "./rdpImport";
import type { HostProfile } from "./types";

function host(patch: Partial<HostProfile>): HostProfile {
  return {
    id: "h1",
    friendlyName: "fudo-server_adeon",
    address: "fudo.example.ch",
    port: 3389,
    protocol: "rdp",
    ...patch,
  } as HostProfile;
}

describe("importedName", () => {
  it("drops a browser's repeat-download counter", () => {
    expect(importedName("fudo-server_adeon (58)")).toBe("fudo-server_adeon");
    expect(importedName("Office PC(2)")).toBe("Office PC");
  });

  it("keeps everything else", () => {
    expect(importedName("Office PC")).toBe("Office PC");
    expect(importedName("Build (x64) box")).toBe("Build (x64) box");
    expect(importedName("(3)")).toBe("(3)");
  });
});

describe("splitRdpUser", () => {
  it("splits DOMAIN\\user", () => {
    expect(splitRdpUser("CORP\\alice")).toEqual({ domain: "CORP", user: "alice" });
  });

  it("leaves a bare or email-style user alone", () => {
    expect(splitRdpUser("80560898UCkDbk")).toEqual({ user: "80560898UCkDbk", domain: "" });
    expect(splitRdpUser("alice@corp.example")).toEqual({ user: "alice@corp.example", domain: "" });
    expect(splitRdpUser(null)).toEqual({ user: "", domain: "" });
    expect(splitRdpUser("\\alice")).toEqual({ user: "\\alice", domain: "" });
  });
});

describe("findExistingHost", () => {
  const hosts = [
    host({}),
    host({ id: "vnc", protocol: "vnc" }),
    host({ id: "other-port", port: 3390 }),
  ];

  it("matches an RDP host by name, address and port", () => {
    const found = findExistingHost(hosts, { address: "FUDO.example.ch ", port: 3389 }, "fudo-server_adeon");
    expect(found?.id).toBe("h1");
  });

  it("does not match another name, port or protocol", () => {
    expect(findExistingHost(hosts, { address: "fudo.example.ch", port: 3389 }, "other")).toBeUndefined();
    expect(findExistingHost([hosts[1]], { address: "fudo.example.ch", port: 3389 }, "fudo-server_adeon")).toBeUndefined();
    expect(findExistingHost(hosts, { address: "fudo.example.ch", port: 3391 }, "fudo-server_adeon")).toBeUndefined();
  });
});
