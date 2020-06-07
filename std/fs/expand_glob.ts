import {
  GlobOptions,
  SEP_PATTERN,
  globToRegExp,
  isAbsolute,
  isGlob,
  joinGlobs,
  normalize,
} from "../path/mod.ts";
import {
  WalkEntry,
  createWalkEntry,
  createWalkEntrySync,
  walk,
  walkSync,
} from "./walk.ts";
import { assert } from "../_util/assert.ts";
const { cwd } = Deno;
type FileInfo = Deno.FileInfo;

const isWindows = Deno.build.os == "windows";

export interface ExpandGlobOptions extends GlobOptions {
  root?: string;
  exclude?: string[];
  includeDirs?: boolean;
}

interface SplitPath {
  segments: string[];
  isAbsolute: boolean;
  hasTrailingSep: boolean;
  // Defined for any absolute Windows path.
  winRoot?: string;
}

// TODO: Maybe make this public somewhere.
function split(path: string): SplitPath {
  const s = SEP_PATTERN.source;
  const segments = path
    .replace(new RegExp(`^${s}|${s}$`, "g"), "")
    .split(SEP_PATTERN);
  const isAbsolute_ = isAbsolute(path);
  return {
    segments,
    isAbsolute: isAbsolute_,
    hasTrailingSep: !!path.match(new RegExp(`${s}$`)),
    winRoot: isWindows && isAbsolute_ ? segments.shift() : undefined,
  };
}

function throwUnlessNotFound(error: Error): void {
  if (!(error instanceof Deno.errors.NotFound)) {
    throw error;
  }
}

function comparePath(a: WalkEntry, b: WalkEntry): number {
  if (a.path < b.path) return -1;
  if (a.path > b.path) return 1;
  return 0;
}

/**
 * Expand the glob string from the specified `root` directory and yield each
 * result as a `WalkEntry` object.
 */
export async function* expandGlob(
  glob: string,
  {
    root = cwd(),
    exclude = [],
    includeDirs = true,
    extended = false,
    globstar = false,
  }: ExpandGlobOptions = {}
): AsyncIterableIterator<WalkEntry> {
  const globOptions: GlobOptions = { extended, globstar };
  const absRoot = isAbsolute(root)
    ? normalize(root)
    : joinGlobs([cwd(), root], globOptions);
  const resolveFromRoot = (path: string): string =>
    isAbsolute(path)
      ? normalize(path)
      : joinGlobs([absRoot, path], globOptions);
  const excludePatterns = exclude
    .map(resolveFromRoot)
    .map((s: string): RegExp => globToRegExp(s, globOptions));
  const shouldInclude = (path: string): boolean =>
    !excludePatterns.some((p: RegExp): boolean => !!path.match(p));
  const { segments, hasTrailingSep, winRoot } = split(resolveFromRoot(glob));

  let fixedRoot = winRoot != undefined ? winRoot : "/";
  while (segments.length > 0 && !isGlob(segments[0])) {
    const seg = segments.shift();
    assert(seg != null);
    fixedRoot = joinGlobs([fixedRoot, seg], globOptions);
  }

  let fixedRootInfo: WalkEntry;
  try {
    fixedRootInfo = await createWalkEntry(fixedRoot);
  } catch (error) {
    return throwUnlessNotFound(error);
  }

  async function* advanceMatch(
    walkInfo: WalkEntry,
    globSegment: string
  ): AsyncIterableIterator<WalkEntry> {
    if (!walkInfo.isDirectory) {
      return;
    } else if (globSegment == "..") {
      const parentPath = joinGlobs([walkInfo.path, ".."], globOptions);
      try {
        if (shouldInclude(parentPath)) {
          return yield await createWalkEntry(parentPath);
        }
      } catch (error) {
        throwUnlessNotFound(error);
      }
      return;
    } else if (globSegment == "**") {
      return yield* walk(walkInfo.path, {
        includeFiles: false,
        skip: excludePatterns,
      });
    }
    yield* walk(walkInfo.path, {
      maxDepth: 1,
      match: [
        globToRegExp(
          joinGlobs([walkInfo.path, globSegment], globOptions),
          globOptions
        ),
      ],
      skip: excludePatterns,
    });
  }

  let currentMatches: WalkEntry[] = [fixedRootInfo];
  for (const segment of segments) {
    // Advancing the list of current matches may introduce duplicates, so we
    // pass everything through this Map.
    const nextMatchMap: Map<string, WalkEntry> = new Map();
    for (const currentMatch of currentMatches) {
      for await (const nextMatch of advanceMatch(currentMatch, segment)) {
        nextMatchMap.set(nextMatch.path, nextMatch);
      }
    }
    currentMatches = [...nextMatchMap.values()].sort(comparePath);
  }
  if (hasTrailingSep) {
    currentMatches = currentMatches.filter(
      (entry: WalkEntry): boolean => entry.isDirectory
    );
  }
  if (!includeDirs) {
    currentMatches = currentMatches.filter(
      (entry: WalkEntry): boolean => !entry.isDirectory
    );
  }
  yield* currentMatches;
}

/** Synchronous version of `expandGlob()`. */
export function* expandGlobSync(
  glob: string,
  {
    root = cwd(),
    exclude = [],
    includeDirs = true,
    extended = false,
    globstar = false,
  }: ExpandGlobOptions = {}
): IterableIterator<WalkEntry> {
  const globOptions: GlobOptions = { extended, globstar };
  const absRoot = isAbsolute(root)
    ? normalize(root)
    : joinGlobs([cwd(), root], globOptions);
  const resolveFromRoot = (path: string): string =>
    isAbsolute(path)
      ? normalize(path)
      : joinGlobs([absRoot, path], globOptions);
  const excludePatterns = exclude
    .map(resolveFromRoot)
    .map((s: string): RegExp => globToRegExp(s, globOptions));
  const shouldInclude = (path: string): boolean =>
    !excludePatterns.some((p: RegExp): boolean => !!path.match(p));
  const { segments, hasTrailingSep, winRoot } = split(resolveFromRoot(glob));

  let fixedRoot = winRoot != undefined ? winRoot : "/";
  while (segments.length > 0 && !isGlob(segments[0])) {
    const seg = segments.shift();
    assert(seg != null);
    fixedRoot = joinGlobs([fixedRoot, seg], globOptions);
  }

  let fixedRootInfo: WalkEntry;
  try {
    fixedRootInfo = createWalkEntrySync(fixedRoot);
  } catch (error) {
    return throwUnlessNotFound(error);
  }

  function* advanceMatch(
    walkInfo: WalkEntry,
    globSegment: string
  ): IterableIterator<WalkEntry> {
    if (!walkInfo.isDirectory) {
      return;
    } else if (globSegment == "..") {
      const parentPath = joinGlobs([walkInfo.path, ".."], globOptions);
      try {
        if (shouldInclude(parentPath)) {
          return yield createWalkEntrySync(parentPath);
        }
      } catch (error) {
        throwUnlessNotFound(error);
      }
      return;
    } else if (globSegment == "**") {
      return yield* walkSync(walkInfo.path, {
        includeFiles: false,
        skip: excludePatterns,
      });
    }
    yield* walkSync(walkInfo.path, {
      maxDepth: 1,
      match: [
        globToRegExp(
          joinGlobs([walkInfo.path, globSegment], globOptions),
          globOptions
        ),
      ],
      skip: excludePatterns,
    });
  }

  let currentMatches: WalkEntry[] = [fixedRootInfo];
  for (const segment of segments) {
    // Advancing the list of current matches may introduce duplicates, so we
    // pass everything through this Map.
    const nextMatchMap: Map<string, WalkEntry> = new Map();
    for (const currentMatch of currentMatches) {
      for (const nextMatch of advanceMatch(currentMatch, segment)) {
        nextMatchMap.set(nextMatch.path, nextMatch);
      }
    }
    currentMatches = [...nextMatchMap.values()].sort(comparePath);
  }
  if (hasTrailingSep) {
    currentMatches = currentMatches.filter(
      (entry: WalkEntry): boolean => entry.isDirectory
    );
  }
  if (!includeDirs) {
    currentMatches = currentMatches.filter(
      (entry: WalkEntry): boolean => !entry.isDirectory
    );
  }
  yield* currentMatches;
}
