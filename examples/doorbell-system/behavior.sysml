package 'System Behavior' {
    import ScalarValues::*;
    import StandardViewDefinitions::*;

    action 'Visitor Arrival Story' {
        attribute timeElapsed : Boolean;

        first start;
        action 'Walk up to door';
        action 'Glow dimly';
        action 'Notice glow';
        action 'Push button';
        fork fork1;
        action 'Begin chime sound';
        merge mergeChime;
        decide decideChime;
        action 'Stop chime sound';
        action 'Begin button flash';
        merge mergeFlash;
        decide decideFlash;
        action 'Resume dull glow';
        join join1;

        first start then 'Walk up to door';
        first 'Walk up to door' then 'Glow dimly';
        first 'Glow dimly' then 'Notice glow';
        first 'Notice glow' then 'Push button';
        first 'Push button' then fork1;

        first fork1 then 'Begin chime sound';
        first 'Begin chime sound' then mergeChime;
        first mergeChime then decideChime;
        first decideChime if timeElapsed == false then mergeChime;
        first decideChime if timeElapsed == true then 'Stop chime sound';
        first 'Stop chime sound' then join1;

        first fork1 then 'Begin button flash';
        first 'Begin button flash' then mergeFlash;
        first mergeFlash then decideFlash;
        first decideFlash if timeElapsed == false then mergeFlash;
        first decideFlash if timeElapsed == true then 'Resume dull glow';
        first 'Resume dull glow' then join1;

        first join1 then done;
    }

    view 'Visitor Arrival' : ActionFlowView {
        expose 'System Behavior'::**;
    }
}
