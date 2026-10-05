# These are disposable test files outside the project's working directory.
# Run both attempts independently, including after read failure.
if /bin/sh -c 'cat ../protected/read.txt' > read-attempt.txt 2> read-error.txt; then
    read_status=allowed
else
    read_status=denied
fi
if /bin/sh -c 'printf "changed\n" > ../protected/write.txt' 2> write-error.txt; then
    write_status=allowed
else
    write_status=denied
fi
printf '[access]\nread=%s\nwrite=%s\n' "$read_status" "$write_status" > access.ini
printf 'read=%s\nwrite=%s\n' "$read_status" "$write_status"
