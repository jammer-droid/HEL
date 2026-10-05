set -eu
# A child shell reads and changes only the disposable project fixture.
/bin/sh -c 'test "$(cat status.txt)" = pending; printf "done\n" > status.txt'
printf 'done\n'
