/**
 * Minimal WASI shim for running hdlfmtr.wasm inside a VSCode extension.
 *
 * This implements only the WASI preview1 functions that hdlfmtr
 * actually calls: fd_read, fd_write, args_get, args_sizes_get,
 * proc_exit, and a handful of stubs. No node:wasi dependency, no
 * experimental flags, works on every Node.js version VSCode ships.
 */

/** Result of running the WASM binary. */
export interface WasiResult {
  code: number;
  stdout: string;
  stderr: string;
}

/** Run hdlfmtr.wasm with the given CLI args and stdin content. */
export async function runWasm(
  wasmBytes: BufferSource,
  args: string[],
  stdin: string
): Promise<WasiResult> {
  const encoder = new TextEncoder();
  const decoder = new TextDecoder();

  const stdinBuf = encoder.encode(stdin);
  let stdinOffset = 0;
  const stdoutChunks: Uint8Array[] = [];
  const stderrChunks: Uint8Array[] = [];
  let exitCode = 0;

  // Encode argv strings as null-terminated bytes.
  const argvBytes = args.map((a) => encoder.encode(a + "\0"));

  const imports: WebAssembly.Imports = {
    wasi_snapshot_preview1: {
      // -- fd operations --------------------------------------------------

      fd_write(
        fd: number,
        iovs: number,
        iovsLen: number,
        nwrittenPtr: number
      ): number {
        const mem = new DataView(memory().buffer);
        let written = 0;
        for (let i = 0; i < iovsLen; i++) {
          const ptr = mem.getUint32(iovs + i * 8, true);
          const len = mem.getUint32(iovs + i * 8 + 4, true);
          const bytes = new Uint8Array(memory().buffer, ptr, len);
          if (fd === 1) {
            stdoutChunks.push(bytes.slice());
          } else if (fd === 2) {
            stderrChunks.push(bytes.slice());
          }
          written += len;
        }
        mem.setUint32(nwrittenPtr, written, true);
        return 0;
      },

      fd_read(
        fd: number,
        iovs: number,
        iovsLen: number,
        nreadPtr: number
      ): number {
        if (fd !== 0) {
          // Only stdin is readable.
          new DataView(memory().buffer).setUint32(nreadPtr, 0, true);
          return 8; // EBADF
        }
        const mem = new DataView(memory().buffer);
        let totalRead = 0;
        for (let i = 0; i < iovsLen; i++) {
          const ptr = mem.getUint32(iovs + i * 8, true);
          const len = mem.getUint32(iovs + i * 8 + 4, true);
          const remaining = stdinBuf.length - stdinOffset;
          const toRead = Math.min(len, remaining);
          if (toRead > 0) {
            new Uint8Array(memory().buffer, ptr, toRead).set(
              stdinBuf.subarray(stdinOffset, stdinOffset + toRead)
            );
            stdinOffset += toRead;
            totalRead += toRead;
          }
          if (remaining <= 0) {
            break;
          }
        }
        mem.setUint32(nreadPtr, totalRead, true);
        return 0;
      },

      fd_close(): number {
        return 0;
      },
      fd_seek(): number {
        return 0;
      },
      fd_fdstat_get(fd: number, bufPtr: number): number {
        // Return a minimal fdstat struct.
        const mem = new DataView(memory().buffer);
        // fs_filetype: 2 = character device
        mem.setUint8(bufPtr, 2);
        // fs_flags: 0
        mem.setUint16(bufPtr + 2, 0, true);
        // fs_rights_base: all
        mem.setBigUint64(bufPtr + 8, BigInt("0xFFFFFFFFFFFFFFFF"), true);
        // fs_rights_inheriting
        mem.setBigUint64(bufPtr + 16, BigInt("0xFFFFFFFFFFFFFFFF"), true);
        return 0;
      },
      fd_prestat_get(): number {
        return 8; // EBADF - no preopened dirs
      },
      fd_prestat_dir_name(): number {
        return 8;
      },

      // -- args -----------------------------------------------------------

      args_sizes_get(argcPtr: number, argvBufSizePtr: number): number {
        const mem = new DataView(memory().buffer);
        mem.setUint32(argcPtr, args.length, true);
        const totalSize = argvBytes.reduce((s, b) => s + b.length, 0);
        mem.setUint32(argvBufSizePtr, totalSize, true);
        return 0;
      },

      args_get(argvPtr: number, argvBufPtr: number): number {
        const mem = new DataView(memory().buffer);
        let bufOffset = argvBufPtr;
        for (let i = 0; i < argvBytes.length; i++) {
          mem.setUint32(argvPtr + i * 4, bufOffset, true);
          new Uint8Array(memory().buffer, bufOffset, argvBytes[i].length).set(
            argvBytes[i]
          );
          bufOffset += argvBytes[i].length;
        }
        return 0;
      },

      // -- env (empty) ----------------------------------------------------

      environ_sizes_get(countPtr: number, sizePtr: number): number {
        const mem = new DataView(memory().buffer);
        mem.setUint32(countPtr, 0, true);
        mem.setUint32(sizePtr, 0, true);
        return 0;
      },

      environ_get(): number {
        return 0;
      },

      // -- clock (stub) ---------------------------------------------------

      clock_time_get(
        _id: number,
        _precision: bigint,
        timePtr: number
      ): number {
        const mem = new DataView(memory().buffer);
        mem.setBigUint64(timePtr, BigInt(Date.now()) * BigInt(1_000_000), true);
        return 0;
      },

      // -- proc -----------------------------------------------------------

      proc_exit(code: number): void {
        exitCode = code;
        throw new WasiExit(code);
      },

      // -- filesystem stubs (we never open files; stdin-only) ---------------

      path_open(): number {
        return 76; // ENOTSUP - filesystem access not available
      },

      fd_filestat_get(fd: number, bufPtr: number): number {
        // Return a zeroed stat struct. Only fd 0/1/2 are real; the
        // binary may probe them during startup.
        const mem = new DataView(memory().buffer);
        for (let i = 0; i < 64; i++) {
          mem.setUint8(bufPtr + i, 0);
        }
        return 0;
      },

      // -- stubs for anything else clap / anyhow might call ---------------
      random_get(bufPtr: number, bufLen: number): number {
        const buf = new Uint8Array(memory().buffer, bufPtr, bufLen);
        for (let i = 0; i < bufLen; i++) {
          buf[i] = Math.floor(Math.random() * 256);
        }
        return 0;
      },
    },
  };

  const module = await WebAssembly.compile(wasmBytes);
  const instance = await WebAssembly.instantiate(module, imports);

  function memory(): WebAssembly.Memory {
    return instance.exports.memory as WebAssembly.Memory;
  }

  try {
    (instance.exports._start as () => void)();
  } catch (e) {
    if (!(e instanceof WasiExit)) {
      throw e;
    }
  }

  const concatChunks = (chunks: Uint8Array[]): string => {
    const total = chunks.reduce((s, c) => s + c.length, 0);
    const buf = new Uint8Array(total);
    let offset = 0;
    for (const c of chunks) {
      buf.set(c, offset);
      offset += c.length;
    }
    return decoder.decode(buf);
  };

  return {
    code: exitCode,
    stdout: concatChunks(stdoutChunks),
    stderr: concatChunks(stderrChunks),
  };
}

class WasiExit extends Error {
  code: number;
  constructor(code: number) {
    super(`WASI exit: ${code}`);
    this.code = code;
  }
}
