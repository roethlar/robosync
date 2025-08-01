# CRITICAL REPOSITORY RULES FOR CLAUDE

## NEVER COMMIT TO GIT:
- Test files or directories (test_*, perf_*, *_test*)
- Temporary files (*.tmp, *.temp, *.log)
- Build artifacts (target/, release/, *.zip, *.tar.gz)
- AI coordination files
- Package manager configs (except when explicitly for distribution)
- Any file that isn't absolutely necessary for users to build the app

## BEFORE EVERY COMMIT:
1. Check `git status` 
2. Review EVERY file being added
3. Ask yourself: "Does a user need this to build the app?"
4. If no, add it to .gitignore FIRST

## REPOSITORY ESSENTIALS ONLY:
- src/ (source code)
- Cargo.toml, Cargo.lock (build config)
- LICENSE, README.md, CHANGELOG.md (documentation)
- .github/workflows/ (CI)
- benches/, tests/, examples/ (if clean and necessary)

## GITIGNORE FIRST PRINCIPLE:
ALWAYS add patterns to .gitignore BEFORE creating files that match those patterns.

## TEST FILES:
NEVER create test files in the repository root. ALWAYS use a temp/ or test/ directory that's already in .gitignore.

## WHEN CLEANING:
- Use `git rm -r --cached` to remove tracked files
- Update .gitignore to prevent re-addition
- Commit both changes together

## PACKAGE MANAGERS:
Keep package manager files LOCAL. They should NEVER be in the public repository unless they're the official distributed package files.