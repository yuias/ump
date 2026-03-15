# Project Guidelines

## Code Style

### Comment Style
- Comments within source code should be written in English by default
- Use clear, concise English for inline comments
- Document public APIs with doc comments (`///` in Rust)
- Keep comments up-to-date with code changes

## Commit Strategy

### Commit Conventions
- Keep commits focused and atomic
- Write clear commit messages using Conventional Commits (https://www.conventionalcommits.org/en/v1.0.0/) style
- The subject MUST BE written in English whenever possible
- For everything else, use clear language, primarily English
- If you ask a user for a JIRA ticket or GitHub Issue number and receive a meaningful response, include `Refs: <Ticket>` in the footer (e.g., if no string is returned, there is no need to include it)

## Branch Strategy
- main: For Production. DO NOT PUSH DIRECTORY.
- feature/<short-description>: Feature Developments
- fix/<short-description>: Bug Fixes
- bugfix/<short-description>: Bug Fixes (same as fix)
- release/<version>: Release Candidates. MUST BE REQURIED PR.

Create these branches from the main branch unless otherwise instructed.  
For example, fix branches for specific feature branches do not need to be create from develop.

## PR (Pull Request) Strategy
- Write subjects using Conventional Commits (https://www.conventionalcommits.org/en/v1.0.0/) style.
- All descriptions should be written in English by default
