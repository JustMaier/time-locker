const fs = require('fs');
const path = require('path');
const { execSync, spawnSync } = require('child_process');
const readline = require('readline');

const TLE_PATH = path.join(__dirname, 'tle.exe');

function parseFrontMatter(content) {
    const match = content.match(/^---\r?\n([\s\S]*?)\r?\n---\r?\n([\s\S]*)$/);
    if (!match) return null;

    const frontMatter = {};
    match[1].split(/\r?\n/).forEach(line => {
        const idx = line.indexOf(':');
        if (idx > 0) {
            const key = line.slice(0, idx).trim();
            let value = line.slice(idx + 1).trim();
            if (value === 'true') value = true;
            else if (value === 'false') value = false;
            frontMatter[key] = value;
        }
    });

    return { frontMatter, body: match[2] };
}

function formatFrontMatter(fm) {
    let out = '---\n';
    for (const [key, value] of Object.entries(fm)) {
        out += `${key}: ${value}\n`;
    }
    out += '---\n';
    return out;
}

function formatDate(date) {
    return date.toISOString().replace('T', ' ').slice(0, 19) + ' UTC';
}

function formatLocalDate(date) {
    return date.toLocaleString(undefined, {
        weekday: 'short',
        year: 'numeric',
        month: 'short',
        day: 'numeric',
        hour: '2-digit',
        minute: '2-digit',
        second: '2-digit'
    });
}

function formatTimeRemaining(ms) {
    if (ms <= 0) return 'now';

    const seconds = Math.floor(ms / 1000);
    const minutes = Math.floor(seconds / 60);
    const hours = Math.floor(minutes / 60);
    const days = Math.floor(hours / 24);
    const months = Math.floor(days / 30);
    const years = Math.floor(days / 365);

    const parts = [];

    if (years > 0) {
        parts.push(`${years} year${years !== 1 ? 's' : ''}`);
        const remainingMonths = Math.floor((days % 365) / 30);
        if (remainingMonths > 0) {
            parts.push(`${remainingMonths} month${remainingMonths !== 1 ? 's' : ''}`);
        }
    } else if (months > 0) {
        parts.push(`${months} month${months !== 1 ? 's' : ''}`);
        const remainingDays = days % 30;
        if (remainingDays > 0) {
            parts.push(`${remainingDays} day${remainingDays !== 1 ? 's' : ''}`);
        }
    } else if (days > 0) {
        parts.push(`${days} day${days !== 1 ? 's' : ''}`);
        const remainingHours = hours % 24;
        if (remainingHours > 0) {
            parts.push(`${remainingHours} hour${remainingHours !== 1 ? 's' : ''}`);
        }
    } else if (hours > 0) {
        parts.push(`${hours} hour${hours !== 1 ? 's' : ''}`);
        const remainingMinutes = minutes % 60;
        if (remainingMinutes > 0) {
            parts.push(`${remainingMinutes} minute${remainingMinutes !== 1 ? 's' : ''}`);
        }
    } else if (minutes > 0) {
        parts.push(`${minutes} minute${minutes !== 1 ? 's' : ''}`);
        const remainingSeconds = seconds % 60;
        if (remainingSeconds > 0) {
            parts.push(`${remainingSeconds} second${remainingSeconds !== 1 ? 's' : ''}`);
        }
    } else {
        parts.push(`${seconds} second${seconds !== 1 ? 's' : ''}`);
    }

    return parts.join(', ');
}

function parseUnlockDate(unlockStr) {
    // Parse "2025-12-20 12:03:21 UTC" format
    const match = unlockStr.match(/^(\d{4}-\d{2}-\d{2}) (\d{2}:\d{2}:\d{2}) UTC$/);
    if (!match) return null;
    return new Date(match[1] + 'T' + match[2] + 'Z');
}

function parseDuration(duration) {
    const match = duration.match(/^(\d+)(ns|us|µs|ms|s|m|h|d|M|y)$/);
    if (!match) return null;

    const num = parseInt(match[1]);
    const unit = match[2];
    const multipliers = {
        'ns': 1e-9, 'us': 1e-6, 'µs': 1e-6, 'ms': 1e-3,
        's': 1, 'm': 60, 'h': 3600, 'd': 86400,
        'M': 86400 * 30, 'y': 86400 * 365
    };

    return num * multipliers[unit] * 1000; // return milliseconds
}

function parseLockTime(input) {
    // First try as a duration (e.g., "5m", "1h", "30d")
    const durationMs = parseDuration(input);
    if (durationMs) {
        const now = new Date();
        return {
            durationStr: input,
            unlockDate: new Date(now.getTime() + durationMs),
            durationMs: durationMs
        };
    }

    // Try as a date: YYYY-MM-DD or YYYY-MM-DD HH:MM or YYYY-MM-DD HH:MM:SS
    const datePatterns = [
        /^(\d{4}-\d{2}-\d{2})$/,                         // 2026-07-01
        /^(\d{4}-\d{2}-\d{2})\s+(\d{1,2}:\d{2})$/,       // 2026-07-01 14:00
        /^(\d{4}-\d{2}-\d{2})\s+(\d{1,2}:\d{2}:\d{2})$/, // 2026-07-01 14:00:00
    ];

    for (const pattern of datePatterns) {
        const match = input.match(pattern);
        if (match) {
            let dateStr = match[1];
            let timeStr = match[2] || '00:00:00';

            // Normalize time string to HH:MM:SS
            const timeParts = timeStr.split(':');
            if (timeParts.length === 2) {
                timeStr = `${timeParts[0].padStart(2, '0')}:${timeParts[1]}:00`;
            } else {
                timeStr = `${timeParts[0].padStart(2, '0')}:${timeParts[1]}:${timeParts[2]}`;
            }

            // Parse as local time
            const unlockDate = new Date(`${dateStr}T${timeStr}`);

            if (isNaN(unlockDate.getTime())) {
                return null;
            }

            const now = new Date();
            const diffMs = unlockDate.getTime() - now.getTime();

            if (diffMs <= 0) {
                return { error: 'Date must be in the future' };
            }

            // Convert to seconds for tle.exe duration
            const diffSeconds = Math.ceil(diffMs / 1000);

            return {
                durationStr: `${diffSeconds}s`,
                unlockDate: unlockDate,
                durationMs: diffMs
            };
        }
    }

    return null;
}

async function prompt(question) {
    const rl = readline.createInterface({
        input: process.stdin,
        output: process.stdout
    });

    return new Promise(resolve => {
        rl.question(question, answer => {
            rl.close();
            resolve(answer);
        });
    });
}

async function encrypt(filePath, content, lockTimeInput) {
    const now = new Date();
    const lockTime = parseLockTime(lockTimeInput);

    if (!lockTime) {
        console.error('Invalid format. Use duration (30s, 5m, 1h, 2d) or date (2026-07-01, 2026-07-01 14:00)');
        process.exit(1);
    }

    if (lockTime.error) {
        console.error(lockTime.error);
        process.exit(1);
    }

    // Encrypt using tle
    const result = spawnSync(TLE_PATH, ['-e', '-D', lockTime.durationStr, '-a'], {
        input: content,
        encoding: 'utf8',
        maxBuffer: 50 * 1024 * 1024
    });

    if (result.status !== 0) {
        console.error('Encryption failed:', result.stderr);
        process.exit(1);
    }

    const frontMatter = {
        locked: true,
        created: formatDate(now),
        unlocks: formatDate(lockTime.unlockDate),
        duration: lockTimeInput,
        original_file: path.basename(filePath)
    };

    const output = formatFrontMatter(frontMatter) + '\n' + result.stdout.trim() + '\n';
    return output;
}

function decrypt(body) {
    const result = spawnSync(TLE_PATH, ['-d'], {
        input: body.trim(),
        encoding: 'utf8',
        maxBuffer: 50 * 1024 * 1024
    });

    return {
        success: result.status === 0,
        output: result.stdout,
        error: result.stderr
    };
}

async function encryptText(text, lockTimeInput, filename) {
    const now = new Date();
    const lockTime = parseLockTime(lockTimeInput);

    if (!lockTime) {
        console.error('Invalid format. Use duration (30s, 5m, 1h, 2d) or date (2026-07-01, 2026-07-01 14:00)');
        process.exit(1);
    }

    if (lockTime.error) {
        console.error(lockTime.error);
        process.exit(1);
    }

    // Encrypt using tle
    const result = spawnSync(TLE_PATH, ['-e', '-D', lockTime.durationStr, '-a'], {
        input: text,
        encoding: 'utf8',
        maxBuffer: 50 * 1024 * 1024
    });

    if (result.status !== 0) {
        console.error('Encryption failed:', result.stderr);
        process.exit(1);
    }

    const frontMatter = {
        locked: true,
        created: formatDate(now),
        unlocks: formatDate(lockTime.unlockDate),
        duration: lockTimeInput,
        original_file: filename || 'typed-message'
    };

    const output = formatFrontMatter(frontMatter) + '\n' + result.stdout.trim() + '\n';
    return output;
}

async function main() {
    const filePath = process.argv[2];

    if (!filePath) {
        // Interactive mode - type a message to encrypt
        console.log('=== TimeLock Encryption ===\n');
        console.log('No file provided. You can type a message to encrypt.\n');

        const message = await prompt('Enter your message: ');
        if (!message.trim()) {
            console.log('No message entered. Exiting.');
            await prompt('\nPress Enter to exit...');
            process.exit(0);
        }

        const duration = await prompt('Enter lock time (e.g., 5m, 1h, 1d, or 2026-07-01): ');
        const filename = await prompt('Enter output filename (without extension, or press Enter for "locked-message"): ');

        const outputName = (filename.trim() || 'locked-message') + '.md';
        const outputPath = path.join(__dirname, outputName);

        const output = await encryptText(message, duration, outputName);
        fs.writeFileSync(outputPath, output, 'utf8');

        console.log(`\nMessage encrypted and saved to: ${outputName}`);
        console.log('Drop this file on tlock.bat again after the unlock time to decrypt.');
        await prompt('\nPress Enter to exit...');
        process.exit(0);
    }

    if (!fs.existsSync(filePath)) {
        console.error('File not found:', filePath);
        await prompt('\nPress Enter to exit...');
        process.exit(1);
    }

    const content = fs.readFileSync(filePath, 'utf8');
    const parsed = parseFrontMatter(content);

    if (parsed && parsed.frontMatter.locked === true) {
        // Try to decrypt
        console.log('Attempting to decrypt...');
        const unlockDateForDisplay = parseUnlockDate(parsed.frontMatter.unlocks);
        if (unlockDateForDisplay) {
            console.log(`This file unlocks at: ${formatLocalDate(unlockDateForDisplay)}`);
        } else {
            console.log(`This file unlocks at: ${parsed.frontMatter.unlocks}`);
        }
        console.log();

        const result = decrypt(parsed.body);

        if (result.success) {
            // Update the file with decrypted content
            const newFrontMatter = { ...parsed.frontMatter };
            newFrontMatter.locked = false;
            newFrontMatter.unlocked = formatDate(new Date());

            const output = formatFrontMatter(newFrontMatter) + '\n' + result.output;
            fs.writeFileSync(filePath, output, 'utf8');

            console.log('Successfully decrypted!');
            console.log('---');
            console.log(result.output);
        } else {
            if (result.error.includes('too early')) {
                const unlockDate = parseUnlockDate(parsed.frontMatter.unlocks);
                const now = new Date();
                const timeRemaining = unlockDate ? unlockDate.getTime() - now.getTime() : 0;

                console.log('Cannot decrypt yet - too early!');
                console.log();
                if (unlockDate) {
                    console.log(`Unlocks at:    ${formatLocalDate(unlockDate)}`);
                    console.log(`Time remaining: ${formatTimeRemaining(timeRemaining)}`);
                } else {
                    console.log(`Come back after: ${parsed.frontMatter.unlocks}`);
                }
            } else {
                console.error('Decryption failed:', result.error);
            }
        }
    } else if (parsed && parsed.frontMatter.locked === false) {
        // Already decrypted, offer to re-encrypt
        console.log('This file has already been decrypted.');
        const answer = await prompt('Would you like to re-encrypt it? (y/n): ');

        if (answer.toLowerCase() === 'y') {
            const duration = await prompt('Enter lock time (e.g., 5m, 1h, 1d, or 2026-07-01): ');
            const output = await encrypt(filePath, parsed.body.trim(), duration);
            fs.writeFileSync(filePath, output, 'utf8');
            console.log(`\nFile encrypted! Will unlock at the time shown in the file.`);
        }
    } else {
        // New file to encrypt
        console.log(`Encrypting: ${path.basename(filePath)}`);
        const duration = await prompt('Enter lock time (e.g., 5m, 1h, 1d, or 2026-07-01): ');

        const output = await encrypt(filePath, content, duration);

        // Save as .md if not already
        let outPath = filePath;
        if (!filePath.endsWith('.md')) {
            outPath = filePath + '.locked.md';
        }

        fs.writeFileSync(outPath, output, 'utf8');
        console.log(`\nFile encrypted and saved to: ${path.basename(outPath)}`);
        console.log('Drop this file on tlock.bat again after the unlock time to decrypt.');
    }

    await prompt('\nPress Enter to exit...');
}

main().catch(err => {
    console.error('Error:', err);
    process.exit(1);
});
