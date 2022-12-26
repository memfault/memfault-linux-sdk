MFLT-XXXX description of the ticket

 ### Summary

 ### Test Plan

# Example Commit Message:
#
# MFLT-XXX Add CI check for commit message formatting
#
#  ### Summary
#
#  Memfault follows the one-idea-is-one-commit philosophy for pull requests.
#  (https://mflt.io/one-idea-one-commit).
#
#  A pull request with a single commit and description about the change made
#  makes it easy for one to quickly get historical context around why a change
#  was made and the problem it solved. It also simplifies tracking down regressions
#  with tools like git bisect.
#
# This PR adds a .gitmessage template as a reference for the commit message format to be used at
# memfault. The template can be installed to pre-populate the commit message:
#
# When "git commit" is invoked:
#  git config commit.template .gitmessage
#
# When "stg new" is invoked:
#  cat .gitmessage  | grep -v "^\#" > .git/patchdescr.tmpl
#
#  ### Test Plan
#
#  Ran install sequences listed above for git (and stacked git) and confirmed
#  commit message template showed up.
