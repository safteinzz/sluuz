#!/bin/bash
for dir in */; do
    echo -e "\n=== $dir ==="
    cd "$dir"
     git branch -r | awk '{print $1}' | xargs git grep -l "afdsafdsa\|324325325\|afdfsd"
     cd ..
done

