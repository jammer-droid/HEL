# Emit enough text to exceed hel's 12,500 estimated-token spill threshold.
# The target is outside the prefix and suffix initially returned to the model.
/usr/bin/awk 'BEGIN {
    for (i = 1; i <= 3000; i++) {
        if (i == 1500) print "TARGET=cedar-4827";
        else printf "row-%04d: abcdefghijklmnopqrstuvwxyz0123456789abcdefghijklmnopqrstuvwxyz0123456789\n", i;
    }
}'
