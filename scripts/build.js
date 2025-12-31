#!/usr/bin/env node

/**
 * Tauri Build Script
 *
 * Builds the Tauri app and optionally renames the executable with version number.
 * Supports cross-platform builds for Windows, Linux, and macOS.
 *
 * Usage:
 *   npm run tauri:build                          - Build without renaming
 *   npm run tauri:build -- --rename              - Build and rename with version
 *   npm run tauri:build -- --rename --target x86_64-pc-windows-msvc
 */

import fs from 'fs';
import { spawn } from 'child_process';
import path from 'path';
import { fileURLToPath } from 'url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const ROOT_DIR = path.join(__dirname, '..');

const CARGO_TOML_PATH = path.join(ROOT_DIR, 'src-tauri', 'Cargo.toml');

/**
 * Parse command line arguments
 */
function parseArgs() {
    const args = process.argv.slice(2);
    const rename = args.includes('--rename');

    const targetIndex = args.indexOf('--target');
    const target = targetIndex !== -1 ? args[targetIndex + 1] : null;

    return { rename, target };
}

/**
 * Get version from Cargo.toml
 */
function getVersion() {
    const content = fs.readFileSync(CARGO_TOML_PATH, 'utf8');
    const match = content.match(/^version\s*=\s*"([\d.]+)"/m);
    if (!match) {
        throw new Error('Could not find version in Cargo.toml');
    }
    return match[1];
}

/**
 * Get platform-specific executable info
 */
function getExecutableInfo(target) {
    const isWindows = target ? target.includes('windows') : process.platform === 'win32';
    const ext = isWindows ? '.exe' : '';
    const originalName = `time-locker${ext}`;
    const versionedPrefix = 'TimeLocker';

    return { isWindows, ext, originalName, versionedPrefix };
}

/**
 * Get the release directory path based on target
 */
function getReleaseDir(target) {
    if (target) {
        return path.join(ROOT_DIR, 'src-tauri', 'target', target, 'release');
    }
    return path.join(ROOT_DIR, 'src-tauri', 'target', 'release');
}

/**
 * Rename the built executable to include version number
 */
function renameExecutable(version, target) {
    const { ext, originalName, versionedPrefix } = getExecutableInfo(target);
    const releaseDir = getReleaseDir(target);

    const originalExe = path.join(releaseDir, originalName);
    const versionedExe = path.join(releaseDir, `${versionedPrefix}-${version}${ext}`);

    if (!fs.existsSync(originalExe)) {
        console.warn(`Warning: Could not find built executable at ${originalExe}`);
        return null;
    }

    // Remove old versioned exe if it exists
    if (fs.existsSync(versionedExe)) {
        fs.unlinkSync(versionedExe);
    }

    fs.copyFileSync(originalExe, versionedExe);
    console.log(`Created versioned executable: ${versionedPrefix}-${version}${ext}`);
    return versionedExe;
}

/**
 * Run the Tauri build
 */
function runTauriBuild(target) {
    return new Promise((resolve, reject) => {
        console.log('Starting Tauri build...\n');

        const isWindows = process.platform === 'win32';
        const npm = isWindows ? 'npm.cmd' : 'npm';

        const args = ['run', 'tauri', '--', 'build'];
        if (target) {
            args.push('--target', target);
        }

        const build = spawn(npm, args, {
            cwd: ROOT_DIR,
            stdio: 'inherit',
            shell: true
        });

        build.on('close', (code) => {
            if (code === 0) {
                resolve();
            } else {
                reject(new Error(`Build failed with exit code ${code}`));
            }
        });

        build.on('error', (error) => {
            reject(error);
        });
    });
}

async function main() {
    const { rename, target } = parseArgs();

    if (target) {
        console.log(`Building for target: ${target}`);
    }

    try {
        await runTauriBuild(target);

        if (rename) {
            const version = getVersion();
            renameExecutable(version, target);
        }

        console.log('\nBuild complete!');
    } catch (error) {
        console.error('\nBuild failed:', error.message);
        process.exit(1);
    }
}

main();
