# Remove WorkBuddy Status Design

## Scope

Remove the WorkBuddy account status feature that queries and displays credits, today's reward, and consecutive check-in days.

Keep WorkBuddy account import, switching and launching, manual check-in, automatic check-in configuration, session watching, settings, and GitHub synchronization.

## Architecture

Delete the dedicated Rust status module and its Tauri command registration. Remove the corresponding Rust and TypeScript status models, frontend service call, Zustand state/actions, automatic refresh/retry effects, account-card metrics, and refresh button.

Add a source-contract regression test that scans the affected files and rejects the removed command, model, store fields, labels, and module registration.

## Success Criteria

- No WorkBuddy request is made to the resource or check-in status endpoints.
- WorkBuddy cards do not show credits, today's reward, consecutive check-in days, or a refresh-status action.
- No WorkBuddy status cache, retry timer, command, model, parser, or related test remains.
- Trae/Work CN credit functionality is unchanged.
